// SPDX-License-Identifier: MPL-2.0

//! The create-object recipes.
//!
//! This module hosts the three create-family recipe methods on
//! [`OverlayInode`]: the create-object dispatcher
//! ([`OverlayInode::create_object`]), the upper-only create
//! ([`OverlayInode::create_upper_only`]), and the create-over-whiteout
//! replacement ([`OverlayInode::create_over_whiteout`], including the
//! opaque-directory branch). The thin `Inode`-trait entries
//! (`create`/`mknod`/`write_link`) and the `DIR` transaction helpers live in
//! the sibling `dir/mod.rs`; the recipes compose the owner helpers inline —
//! `project_new_upper` + `BindingCache::insert` + `readdir_index_insert`.
//!
//! Lock domains: `DIR` = per-parent directory transaction lock; `CUL` =
//! per-object copy-up lock; `INODE` = per-object facts lock; `WL` =
//! whiteout-cache lock; `MOUNT` = mount-lifecycle lock; `UPPER` =
//! underlying upper-filesystem lock; `IU` = mount-time upper/workdir
//! in-use claim.
//!
//! Lock contract: the caller (the `dir/mod.rs` entry) holds the parent `DIR`
//! transaction lock and has pinned the mount. This module acquires no Overlay
//! lock of its own beyond the brief `INODE` facts snapshots inside
//! `facts_snapshot`/`select_real_inode` (snapshot-and-release, never held
//! across an underlying call) and the `CUL` domain entered inside the real
//! stage of `check_permission(AccessType::Mutating, ...)` (promotes the
//! parent under the caller-held `DIR`). Upper/workdir physical operations run
//! through the base VFS `Path` layer in the sleep-capable `DIR` domain under
//! the underlying filesystem's own locking; no `WL`/spin domain is entered
//! and no `WL` payload is touched (the whiteout cache is the sibling
//! `dir/whiteout.rs` owner).
//!
//! The two recipe methods are private to this module (their only caller is
//! `create_object` in this file).

use crate::{
    fs::{
        file::{InodeMode, InodeType, Permission},
        fs_impls::overlayfs::{
            AccessType,
            copyup::WorkdirTempRequest,
            metadata_security::xattr::{OPAQUE_MARKER_VALUE, OPAQUE_XATTR_FULL_NAME},
            mount::{OverlayFs, RealPath},
            projection::{
                Binding, NegativeBinding, OverlayInode, OverlayObjectFacts, PositiveBinding,
                PositiveKind, RealObject,
            },
        },
        vfs::{
            inode::{MknodType, RenameMode},
            xattr::{XattrName, XattrSetFlags},
        },
    },
    prelude::*,
};

impl OverlayInode {
    /// Dispatches one create-family request (create/mkdir/mknod/symlink)
    /// from the fresh `(parent, name)` projection under the parent `DIR`.
    ///
    /// The decision uses current BindingCache/barrier evidence via
    /// `lookup_binding` — never the stale VFS negative dentry that may have
    /// triggered the call:
    ///
    /// - `Negative(Absent)` / `Negative(HiddenByOpaque(_))` → upper-only
    ///   create (`create_upper_only`), no workdir, no opaque marker;
    /// - `Negative(HiddenByWhiteout(_))` → create-over-whiteout
    ///   (`create_over_whiteout`), workdir temp + atomic replace (+ the
    ///   opaque branch when the requested kind is `Dir`);
    /// - `Positive(_)` → `Err(EEXIST)` — a visible lower/merged target is
    ///   never silently replaced.
    ///
    /// # Request shape
    ///
    /// The create request is carried as the `Inode`-trait arguments rather
    /// than a new enum: `type_` + `mode` (the `create` entry shape) plus
    /// `mknod_type: Option<MknodType>` (the `mknod` entry shape; `Some`
    /// selects the mknod recipe at the upper call). The `dir/mod.rs` `mknod`
    /// entry applies the raw-`0:0` gate (`CharDevice(0)` → `EPERM`) before
    /// delegating.
    pub(super) fn create_object(
        &self,
        name: &str,
        type_: InodeType,
        mode: InodeMode,
        mknod_type: Option<MknodType>,
    ) -> Result<Arc<OverlayInode>> {
        let fs = self.fs_arc()?;
        let parent_facts = self.facts_snapshot();
        let binding = fs.lookup_binding(&parent_facts, name)?.binding;
        match binding {
            Binding::Negative(NegativeBinding::Absent)
            | Binding::Negative(NegativeBinding::HiddenByOpaque(_)) => {
                self.create_upper_only(name, type_, mode, mknod_type)
            }
            Binding::Negative(NegativeBinding::HiddenByWhiteout(_)) => {
                self.create_over_whiteout(name, type_, mode, mknod_type)
            }
            Binding::Positive(_) => Err(Error::with_message(
                Errno::EEXIST,
                "the overlay target already exists and is visible",
            )),
        }
    }

    /// Creates a genuinely absent object directly in the upper parent.
    ///
    /// Runs the real admission stage (`check_permission(AccessType::Mutating,
    /// MAY_WRITE)`) — which promotes this parent to upper authority under the
    /// caller-held `DIR` — then performs the upper `create`/`mknod` directly
    /// through the base VFS `Path` layer (no workdir) and publishes the
    /// result inline. The returned dentry-anchored `Path` feeds
    /// [`RealObject::with_path`], so the published object remains
    /// base-view coherent. A plain-absent or opaque-hidden target
    /// never creates opaque. The publication steps are infallible, so no
    /// reconcile arm is structurally reachable in this recipe; the
    /// post-physical failure reconcile lives in
    /// [`OverlayInode::create_over_whiteout`] (the one create-family recipe
    /// with a fallible step after the upper commit).
    fn create_upper_only(
        &self,
        name: &str,
        type_: InodeType,
        mode: InodeMode,
        mknod_type: Option<MknodType>,
    ) -> Result<Arc<OverlayInode>> {
        // The real admission stage: promotes the parent under the caller-held
        // DIR; the EROFS gate + local DAC is the entry's admission, so no
        // EROFS check is duplicated here.
        self.check_permission(AccessType::Mutating, Permission::MAY_WRITE)?;
        let fs = self.fs_arc()?;
        let upper_parent_path = self.upper_parent_path()?;
        // The overlay-visible object kind of the request (used by the index
        // below). Shared mechanical mapping (the `MknodType` ->
        // `InodeType` classification is the single `super::mknod_object_type`
        // helper in `dir/mod.rs`, consumed by all three sites); the `None`
        // leg keeps the plain `create` object type.
        let object_type = mknod_type
            .as_ref()
            .map(super::mknod_object_type)
            .unwrap_or(type_);
        // Upper physical operation: direct create/mknod in the upper parent
        // through the base VFS `Path` layer; the returned `Path` is the
        // dentry-anchored published upper object.
        let new_upper_path = match mknod_type {
            Some(mknod) => upper_parent_path.mknod(name, mode, mknod)?,
            None => upper_parent_path.new_fs_child(name, type_, mode)?,
        };
        // Semantic publication — inline composition: the new upper
        // object's facts, the projected OverlayInode, the positive binding,
        // and the index.
        let upper_layer = fs.layer_stack().upper.as_ref().ok_or_else(|| {
            Error::with_message(Errno::EROFS, "the overlay mount has no upper layer")
        })?;
        let new_facts = OverlayObjectFacts::try_new(
            PositiveKind::Single,
            Some(RealObject::with_path(
                0,
                RealPath::from_path(&new_upper_path),
                upper_layer.fsid,
                upper_layer.container_dev_id,
            )),
            Vec::new(),
        )
        .ok_or_else(|| {
            Error::with_message(
                Errno::EIO,
                "the new upper object facts are not constructible",
            )
        })?;
        let inode = fs.project_new_upper(&new_facts);
        self.publish_positive_binding(&fs, name, inode.clone(), object_type);
        Ok(inode)
    }

    /// Replaces a whiteout-hidden name with a completely prepared private
    /// workdir temp, then publishes it.
    ///
    /// The replacement object is prepared in the workdir (never visible as a
    /// lookup/readdir source), the opaque marker is applied to a `Dir` temp
    /// **before** the atomic swap (the opaque record is part of the
    /// replacement object's complete publication), and the whiteout is
    /// consumed atomically: `Replace` for non-directories, `Exchange` +
    /// workdir unlink of the displaced whiteout for directories. A `SymLink`
    /// temp's target is filled later by the VFS-wide `write_link` two-step.
    /// Publication is the same inline sequence as
    /// [`OverlayInode::create_upper_only`].
    ///
    /// Failure handling: any failure before the atomic upper commit
    /// best-effort-cleans the temp; a failure after the commit (the only
    /// fallible step there is the directory `Exchange`-leg unlink of the
    /// displaced whiteout) reconciles the affected `(parent, name)` =
    /// `(self, name)` projection as a unit via the shared
    /// [`OverlayInode::invalidate_stale_cache`] entry. This arm covers only
    /// the one affected pair of this recipe and never a partial sequence.
    ///
    /// The shared workdir-temp request carries a borrowed [`MknodType`] for
    /// the special-object leg. Its retry owner recreates the VFS value for
    /// each attempt, so device identity survives an `EEXIST` retry without a
    /// caller-local staging operation.
    fn create_over_whiteout(
        &self,
        name: &str,
        type_: InodeType,
        mode: InodeMode,
        mknod_type: Option<MknodType>,
    ) -> Result<Arc<OverlayInode>> {
        // The real admission stage (promotes the parent under the
        // caller-held DIR; the EROFS gate + local DAC is the entry's
        // admission).
        self.check_permission(AccessType::Mutating, Permission::MAY_WRITE)?;
        let fs = self.fs_arc()?;
        let upper_parent_path = self.upper_parent_path()?;
        // Shared mechanical kind mapping (consumed by the opaque branch and
        // the index integration point). Computed before `mknod_type` is consumed by the
        // temp creation below.
        let object_type = mknod_type
            .as_ref()
            .map(super::mknod_object_type)
            .unwrap_or(type_);
        // Private staging: the temp is never a lookup/readdir/ReaddirIndex
        // source. The typed request selects either the `mknod` or create
        // operation while the shared owner performs every retry.
        let temp = match &mknod_type {
            Some(node) => fs.create_workdir_temp(
                name,
                &upper_parent_path,
                WorkdirTempRequest::Mknod { mode, node },
            )?,
            None => fs.create_workdir_temp(
                name,
                &upper_parent_path,
                WorkdirTempRequest::Create { kind: type_, mode },
            )?,
        };
        let temp_kind = temp.kind();
        let (temp_name, temp) = temp.into_parts();
        let workdir_path = self.workdir_root_path()?;
        // The shared recipe scaffold: the commit marker is flipped at the
        // physical upper commit point and the reconcile / pre-publication
        // cleanup classification is owned by `run_recipe` (the staged temp's
        // request-derived kind makes the pre-commit cleanup dir-aware).
        self.run_recipe(
            &fs,
            Some((&temp_name, temp_kind)),
            || self.invalidate_stale_cache(&[(self, name)]),
            |marker| {
                if object_type == InodeType::Dir {
                    // Opaque branch: the opaque record is part of the
                    // replacement directory's complete publication; the
                    // marker write is gated by the private-xattr capability
                    // and runs on the temp before the atomic swap — the
                    // whiteout is never deleted first.
                    let can_store_private_xattr = fs
                        .policy()
                        .upper_capabilities()
                        .is_some_and(|caps| caps.can_store_private_xattr());
                    if !can_store_private_xattr {
                        return Err(Error::with_message(
                            Errno::EOPNOTSUPP,
                            "the upper filesystem cannot store the opaque marker \
                             required for a directory over a whiteout",
                        ));
                    }
                    let marker_name = XattrName::try_from_full_name(OPAQUE_XATTR_FULL_NAME)
                        .ok_or_else(|| {
                            Error::with_message(
                                Errno::EINVAL,
                                "invalid overlay opaque marker xattr name",
                            )
                        })?;
                    let mut marker_reader = VmReader::from(OPAQUE_MARKER_VALUE).to_fallible();
                    temp.set_xattr(
                        marker_name,
                        &mut marker_reader,
                        XattrSetFlags::CREATE_OR_REPLACE,
                    )?;
                }
                // Atomic replacement over the whiteout: `Replace` for non-dirs;
                // for dirs `Exchange` (the displaced whiteout lands in the
                // workdir) then the workdir unlink removes it.
                if object_type.is_directory() {
                    workdir_path.rename(
                        &temp_name,
                        &upper_parent_path,
                        name,
                        RenameMode::Exchange,
                    )?;
                    marker.commit();
                    workdir_path.unlink(&temp_name)?;
                } else {
                    workdir_path.rename(
                        &temp_name,
                        &upper_parent_path,
                        name,
                        RenameMode::Replace,
                    )?;
                    marker.commit();
                }
                // Semantic publication — inline composition. The temp
                // handle is the object now published at `(upper_parent_path,
                // name)` (inode identity is stable across the rename), so it
                // is the new upper real object.
                let upper_layer = fs.layer_stack().upper.as_ref().ok_or_else(|| {
                    Error::with_message(Errno::EROFS, "the overlay mount has no upper layer")
                })?;
                let new_facts = OverlayObjectFacts::try_new(
                    PositiveKind::Single,
                    Some(RealObject::with_path(
                        0,
                        RealPath::from_path(&temp),
                        upper_layer.fsid,
                        upper_layer.container_dev_id,
                    )),
                    Vec::new(),
                )
                .ok_or_else(|| {
                    Error::with_message(
                        Errno::EIO,
                        "the new upper object facts are not constructible",
                    )
                })?;
                let inode = fs.project_new_upper(&new_facts);
                self.publish_positive_binding(&fs, name, inode.clone(), object_type);
                Ok(inode)
            },
        )
    }

    /// Publishes a freshly created upper object as the positive binding of
    /// `(self, name)` and records it in the readdir index — the semantic
    /// publication path shared by the two create recipes; `link_impl`
    /// composes the same two steps inline in `dir/mod.rs`.
    ///
    /// The `BindingCache::insert` half reuses the shared
    /// [`OverlayFs::publish_binding`] integration point (projection/mod.rs) so the
    /// publication key is constructed in one place; the
    /// `readdir_index_insert` half keeps the `(name, inode)` index entry in
    /// sync with the binding.
    fn publish_positive_binding(
        &self,
        fs: &OverlayFs,
        name: &str,
        inode: Arc<OverlayInode>,
        kind: InodeType,
    ) {
        fs.publish_binding(
            &self.key(),
            name,
            Binding::Positive(PositiveBinding::new(inode.clone())),
        );
        self.readdir_index_insert(name, inode, kind);
    }
}
