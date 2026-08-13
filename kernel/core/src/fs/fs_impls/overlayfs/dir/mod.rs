// SPDX-License-Identifier: MPL-2.0

//! The module root of the overlayfs namespace mutation and whiteout
//! subsystem.
//!
//! This module declares the five `dir/*` submodules and hosts the thin
//! `Inode`-trait mutation entries — `create` / `mknod` / `write_link` /
//! `link` / `unlink` / `rmdir` / `rename` — plus the four orchestration
//! helpers in this module: the one-parent `DIR` lock helper
//! ([`OverlayInode::lock_dir_transaction`]), the two-parent `DIR` lock helper
//! ([`OverlayInode::lock_parent_dir_transactions`], stable object-identity
//! order, each parent exactly once), the post-promotion upper real parent
//! `Path` resolution ([`OverlayInode::upper_parent_path`]), and the single
//! shared private reconcile entry ([`OverlayInode::invalidate_stale_cache`]).
//! The real control flow lives in the sibling files: `create.rs` (dispatcher
//! + upper-only/over-whiteout/opaque branches), `remove.rs` (unlink/rmdir +
//!   visible emptiness + clear-empty), `link.rs` (source promotion +
//!   target-over-whiteout fragments), `rename.rs` (EXDEV gate + upper rename +
//!   dual-parent publication), and `whiteout.rs` (whiteout cache +
//!   representation).
//!
//! Lock domains: `DIR` = per-parent directory transaction lock; `CUL` =
//! per-object copy-up lock; `INODE` = per-object facts lock; `WL` =
//! whiteout-cache lock; `MOUNT` = mount-lifecycle lock; `UPPER` =
//! underlying upper-filesystem lock; `IU` = mount-time upper/workdir
//! in-use claim.
//!
//! Lock contract: every mutation entry establishes the affected parent `DIR`
//! domain(s) first (via the two lock helpers below), then runs the mutating
//! permission admission per affected parent under the held `DIR` — the EROFS
//! gate first, then the copy-up promotion in the `DIR -> CUL -> INODE` order
//! — then delegates to the recipe under the same guard(s). All `DIR`/`CUL`/
//! `INODE` domains are released before any VFS-visible return; `MOUNT` is
//! never acquired and `WL` is never acquired by this module (the
//! `whiteout.rs` slot protocol is the only `WL` holder). No
//! `.unwrap()`/`.expect()` appears in any production path (hard invariant
//! failures use the `unreachable!` precedent of `projection/inode.rs`).

use self::remove::RemoveKind;
use super::{
    AccessType,
    projection::{Binding, BindingKey, NegativeBinding, OverlayInode, PositiveBinding},
};
use crate::{
    fs::{
        file::{InodeMode, InodeType, Permission},
        vfs::{
            inode::{Inode, MknodType, RenameMode},
            path::Path,
        },
    },
    prelude::*,
    process::credentials::capabilities::CapSet,
};

pub(super) mod whiteout;

mod create;
mod link;
mod remove;
mod rename;

/// Maps the `mknod` kind request to the overlay-visible object type.
///
/// The `MknodType` -> `InodeType` classification was inlined at three sites
/// (`dir/mod.rs::mknod`, `dir/create.rs::create_upper_only`, and
/// `dir/create.rs::create_over_whiteout`); this helper is the single
/// mechanical mapping — the classification appears at three call sites, so
/// it is factored into this single helper.
/// `MknodType` has no `InodeType` conversion, so the match is the owned
/// mapping.
pub(super) fn mknod_object_type(mknod: &MknodType) -> InodeType {
    match mknod {
        MknodType::NamedPipe => InodeType::NamedPipe,
        MknodType::CharDevice(_) => InodeType::CharDevice,
        MknodType::BlockDevice(_) => InodeType::BlockDevice,
    }
}

impl OverlayInode {
    // create/mkdir/symlink carry the same VFS entry
    // (`Path::new_fs_child` -> `Dentry::create` -> `Inode::create`, verified
    // `syscall/mkdir.rs:38` and `syscall/symlink.rs:44`); the symlink target
    // is filled by the later `write_link` delegation. Admission under the
    // parent `DIR`; the dispatcher (create.rs) re-derives the fresh binding,
    // runs the upper-only/over-whiteout/opaque recipe and its inline
    // publication, and returns the projected `OverlayInode`.
    pub(in crate::fs::fs_impls::overlayfs) fn create_impl(
        &self,
        name: &str,
        type_: InodeType,
        mode: InodeMode,
    ) -> Result<Arc<dyn Inode>> {
        // The parent `DIR` domain is established first so the admission's
        // promotion stage runs under the held `DIR` (`DIR -> CUL`); a failed
        // admission produces no upper/workdir/cache side effect.
        let _dir_guard = self.lock_dir_transaction();
        self.check_permission(AccessType::Mutating, Permission::MAY_WRITE)?;
        // The dispatcher takes the `Inode`-trait create arguments directly
        // (the create request is carried as the trait arguments, not a new
        // enum).
        let projected: Arc<dyn Inode> = self.create_object(name, type_, mode, None)?;
        Ok(projected)
    }

    // A raw `0:0` whiteout char device is refused before any admission or
    // side effect (Linux `ovl_mknod`, dir.c:746-753); every other `mknod`
    // kind funnels into the same dispatcher with the `MknodType` carried in
    // the request.
    pub(in crate::fs::fs_impls::overlayfs) fn mknod_impl(
        &self,
        name: &str,
        mode: InodeMode,
        type_: MknodType,
    ) -> Result<Arc<dyn Inode>> {
        if matches!(&type_, MknodType::CharDevice(0)) {
            return_errno_with_message!(
                Errno::EPERM,
                "a raw 0:0 whiteout char device must not be user-creatable"
            );
        }
        let _dir_guard = self.lock_dir_transaction();
        self.check_permission(AccessType::Mutating, Permission::MAY_WRITE)?;
        // The dispatcher takes the `Inode`-trait create arguments directly:
        // the mechanical `MknodType` -> `InodeType` classification is the
        // shared `mknod_object_type` mapping and the `MknodType` itself is
        // the `mknod_type` leg (the create.rs recipes derive their own
        // object-type classification from `mknod_type` when it is `Some`).
        let object_type = mknod_object_type(&type_);
        let projected: Arc<dyn Inode> = self.create_object(name, object_type, mode, Some(type_))?;
        Ok(projected)
    }

    // The upper symlink was created by `create` (the VFS-wide
    // create-then-`write_link` two-step, syscall/symlink.rs:44-45 — the same
    // window ramfs accepts). Thin delegation to the current authority with no
    // promotion: the created symlink is already upper-backed.
    pub(in crate::fs::fs_impls::overlayfs) fn write_link_impl(&self, target: &str) -> Result<()> {
        self.select_real_inode().write_link(target)
    }

    // Link. Only the target parent's `DIR` is acquired (the source is an
    // object, not a namespace of this mutation; its promotion is
    // `CUL`-serialized). The two `link.rs` fragments — `link_source` (source
    // promotion to the shared upper real `Path`) and `link_over_whiteout`
    // (workdir hard link + rename-over) — are composed here with the fresh
    // target projection and the inline target publication (the publication
    // steps are infallible, so the reconcile path is unreachable here).
    pub(in crate::fs::fs_impls::overlayfs) fn link_impl(
        &self,
        old: &Arc<dyn Inode>,
        name: &str,
    ) -> Result<()> {
        let _dir_guard = self.lock_dir_transaction();
        self.check_permission(AccessType::Mutating, Permission::MAY_WRITE)?;
        let fs = self.fs_arc()?;
        // Fresh target projection under `DIR` (never from a stale VFS
        // dentry): a visible target is never silently replaced.
        let binding = fs.lookup_binding(&self.facts_snapshot(), name)?.binding;
        if matches!(&binding, Binding::Positive(_)) {
            return Err(Error::new(Errno::EEXIST));
        }
        let target_is_whiteout = matches!(
            &binding,
            Binding::Negative(NegativeBinding::HiddenByWhiteout(_))
        );
        // The source must be an Overlay inode (the VFS passes an inode of
        // this filesystem); a foreign inode is a defensive error, never a
        // silent cast.
        let old_overlay = Arc::downcast::<OverlayInode>(old.clone()).map_err(|_| {
            Error::with_message(Errno::EIO, "the link source is not an overlay inode")
        })?;
        // Source-side admission — a mirror of the base VFS
        // `check_hardlink_source` (`kernel/src/fs/vfs/path/mod.rs:281-330`,
        // Linux `may_linkat` under `fs.protected_hardlinks`): the `link`
        // syscall performs no source check of its own (VFS gap), so before
        // the source promotion trigger runs, the caller must either own the
        // source or hold read+write DAC on it; non-owners may only link
        // regular files; and non-owners linking a setuid or executable-setgid
        // source need `CAP_FOWNER`. The read-only admission surface (the
        // 2-param `check_permission` leg — never promotes) evaluated against
        // the source's projected metadata mirrors the base checks; this gates
        // `link_source`'s copy-up, so an inaccessible source is refused with
        // `EPERM` and never forced to the upper layer. The base
        // `check_hardlink_source` remains as the second gate.
        let source_metadata = old_overlay.metadata()?;
        // The owner probe runs through the shared `current_fsuid()`
        // accessor: the kernel-context default — no task / no posix thread
        // means "not the owner" — is handled in one place (`permission.rs`),
        // and the borrow-lifetime trap of binding the owned `CurrentTask`
        // locally is gone.
        let source_owned =
            OverlayInode::current_fsuid().is_some_and(|fsuid| fsuid == source_metadata.uid);
        // Non-owner source: mirror the base `check_hardlink_source`
        // rejections — read+write DAC demand, regular-file-only, and
        // setuid/setgid+group-exec sources require `CAP_FOWNER` via the
        // shared capability helper (kernel contexts fail open, matching the
        // base no-posix-thread `Ok(())`).
        if !source_owned {
            if old_overlay
                .check_permission(
                    AccessType::ReadOnly,
                    Permission::MAY_READ | Permission::MAY_WRITE,
                )
                .is_err()
            {
                return Err(Error::with_message(
                    Errno::EPERM,
                    "the link source is not accessible to the caller",
                ));
            }
            if old_overlay.type_() != InodeType::File {
                return Err(Error::with_message(
                    Errno::EPERM,
                    "the link source is not a regular file",
                ));
            }
            if (source_metadata.mode.has_set_uid()
                || (source_metadata.mode.has_set_gid()
                    && source_metadata.mode.is_group_executable()))
                && !OverlayInode::current_task_has_capability(CapSet::FOWNER)
            {
                return Err(Error::with_message(
                    Errno::EPERM,
                    "the link source is set-id and the caller lacks CAP_FOWNER",
                ));
            }
        }
        // Source promotion (link.rs): `ensure_upper_authority` then the
        // shared upper real object's dentry-anchored `Path`. Without an
        // origin/index, two lower aliases of one lower inode may copy up
        // separately to distinct upper inodes (an acknowledged limitation;
        // future origin/index tracking would remove it); upper-authoritative
        // sources always share one
        // upper inode (real hard link).
        let source_path = self.link_source(&old_overlay)?;
        // Linking an origin-bearing source into this parent makes the parent
        // impure — persist the marker before either physical-link branch
        // (Linux `ovl_create_or_link` origin arm; strict, pre-commit).
        if !old_overlay.facts_snapshot().lowers().is_empty() {
            fs.xattr_policy()
                .set_impure_marker(self.upper_parent_path()?.inode())?;
        }
        if target_is_whiteout {
            // Target hidden by a whiteout: workdir hard link + rename-over
            // the whiteout (Linux `ovl_create_over_whiteout` hardlink leg).
            self.link_over_whiteout(name, &source_path)?;
        } else {
            // Absent (or opaque-hidden) target: direct upper hard link
            // through the base VFS `Path` layer.
            self.upper_parent_path()?.link(&source_path, name)?;
        }
        // Inline target publication: the positive binding shares the source
        // `OverlayInode` — inode-cache reuse by `RealObjectKey`, so
        // `project_new_upper` is not needed — and the readdir-index decision
        // integration point maintains the target parent index (Valid + upper-only rule).
        // Both steps are infallible; they run under the held `DIR` before
        // release.
        let key = BindingKey::new(self.key(), String::from(name));
        let binding = Arc::new(Binding::Positive(PositiveBinding::new(old_overlay.clone())));
        fs.bindings().insert(key, binding);
        self.readdir_index_insert(name, old_overlay.clone(), old_overlay.type_());
        Ok(())
    }

    // Unlink. The `remove.rs` recipe owns the fresh target projection
    // (ENOENT), the pure-upper direct unlink vs lower-backed whiteout
    // publish, and the inline publication (BindingCache invalidate /
    // HiddenByWhiteout insert + `readdir_index_remove`).
    pub(in crate::fs::fs_impls::overlayfs) fn unlink_impl(&self, name: &str) -> Result<()> {
        let _dir_guard = self.lock_dir_transaction();
        self.check_permission(AccessType::Mutating, Permission::MAY_WRITE)?;
        self.remove_target(name, RemoveKind::Unlink)
    }

    // Rmdir. Same admission + single-parent `DIR` shape as `unlink`;
    // `remove_target(name, RemoveKind::Rmdir)` runs the overlay-visible
    // emptiness gate (`visible_child_count`; whiteout-hidden children do not
    // count) before any upper removal and takes the clear-empty path for
    // lower-backed directories with upper children. The operation kind is
    // the closed `RemoveKind` vocabulary (no `is_dir` boolean at the call
    // sites).
    pub(in crate::fs::fs_impls::overlayfs) fn rmdir_impl(&self, name: &str) -> Result<()> {
        let _dir_guard = self.lock_dir_transaction();
        self.check_permission(AccessType::Mutating, Permission::MAY_WRITE)?;
        self.remove_target(name, RemoveKind::Rmdir)
    }

    // Rename. Two-parent `DIR` acquisition (stable object-identity order,
    // each parent exactly once), mutating admission per affected parent, the
    // fresh source projection, the EXDEV gate before any upper side effect,
    // and then the upper rename recipe (`rename_upper`, rename.rs) which
    // owns per-branch promotion, the physical upper rename (+ source-whiteout
    // compose), the dual-parent inline publication, and the reconcile on
    // failure.
    pub(in crate::fs::fs_impls::overlayfs) fn rename_impl(
        &self,
        old_name: &str,
        target: &Arc<dyn Inode>,
        new_name: &str,
        mode: RenameMode,
    ) -> Result<()> {
        // The destination parent must be an Overlay inode (the VFS passes
        // the destination directory of this filesystem); a foreign inode is
        // a defensive error, never a silent cast.
        let target_overlay = Arc::downcast::<OverlayInode>(target.clone()).map_err(|_| {
            Error::with_message(Errno::EIO, "the rename target is not an overlay inode")
        })?;
        // Two-parent `DIR` acquisition in stable object-identity order: the
        // same-parent case returns one guard (each parent exactly once).
        let (_source_guard, _target_guard) =
            self.lock_parent_dir_transactions(Some(&target_overlay))?;
        // Mutating admission per affected parent under the held `DIR`s (EROFS
        // gate first, then the promotion in `DIR -> CUL` order).
        self.check_permission(AccessType::Mutating, Permission::MAY_WRITE)?;
        target_overlay.check_permission(AccessType::Mutating, Permission::MAY_WRITE)?;
        let fs = self.fs_arc()?;
        // Fresh source projection under `DIR`: the `DIR`-domain projection is
        // authoritative over a stale VFS dentry; a negative source is ENOENT.
        let source_binding = fs.lookup_binding(&self.facts_snapshot(), old_name)?.binding;
        let _source_inode = match &source_binding {
            Binding::Positive(positive) => positive.inode(),
            Binding::Negative(_) => return Err(Error::new(Errno::ENOENT)),
        };
        // EXDEV gate before any upper side effect: only a cross-directory
        // move of a lower-backed/merged directory hits the EXDEV default
        // (redirect is not implemented). The same-parent comparison
        // is the `Arc::as_ptr` identity of the two `DIR` lock helpers.
        if !core::ptr::addr_eq(core::ptr::from_ref(self), Arc::as_ptr(&target_overlay)) {
            self.cross_device_gate(&source_binding)?;
        }
        // "Source has a lower fallback" decides whether the source name gets
        // a whiteout after the move; rename.rs derives it internally from the
        // fresh source projection (no bare boolean crosses the entry
        // boundary).
        self.rename_upper(old_name, &target_overlay, new_name, mode)
    }
}

impl OverlayInode {
    /// Returns the payload-less parent `DIR` transaction guard of this
    /// directory.
    ///
    /// The one-parent `DIR` lock helper. `self.dir()` is `Some` exactly for
    /// directory inodes, and every mutation entry of this module is a
    /// child-name operation that the VFS routes on directory inodes (the same
    /// `Some`-invariant `lookup` relies on), so the `None` arm is a hard
    /// invariant failure — never a silent guard-less mutation, and never a
    /// `.unwrap()`/`.expect()`.
    pub(super) fn lock_dir_transaction(&self) -> MutexGuard<'_, ()> {
        match self.dir() {
            Some(dir) => dir.lock(),
            None => unreachable!(
                "mutation entries run on overlay directories only; the VFS routes child-name \
                 operations on directory inodes"
            ),
        }
    }

    /// Acquires the two affected parent `DIR` domains in stable
    /// object-identity order, each parent exactly once.
    ///
    /// The two-parent `DIR` lock helper. The primary ordering key
    /// `RealObjectKey`
    /// lexicographic `(fsid, real_ino)` is not currently publishable — the
    /// landed `RealObjectKey` derives no `Ord` and its fields are
    /// `projection`-private — so this helper applies the chosen alternative
    /// (`Arc::as_ptr` ordering): the two parents are ordered by their `Arc` pointer
    /// address, `core::ptr::from_ref(self)` being exactly the address
    /// `Arc::as_ptr` returns for the same inode. The inode cache
    /// (`get_or_create` by `RealObjectKey`) guarantees one inode per logical
    /// directory, so the address is a stable per-directory identity and the
    /// same-inode case (a same-directory rename) acquires the single `DIR`
    /// once. The guards are returned as the anonymous tuple `(self_guard,
    /// other_guard)` — a local return shape, not a named coordination type;
    /// the elided `'_` lifetimes are written as explicit `'a`/`'b` because the
    /// two guards borrow from two distinct inputs.
    pub(super) fn lock_parent_dir_transactions<'a, 'b>(
        &'a self,
        other: Option<&'b Arc<OverlayInode>>,
    ) -> Result<(MutexGuard<'a, ()>, Option<MutexGuard<'b, ()>>)> {
        let self_dir = match self.dir() {
            Some(dir) => dir,
            None => {
                return Err(Error::with_message(
                    Errno::ENOTDIR,
                    "the source parent is not an overlay directory",
                ));
            }
        };
        // Single-parent operation: the first element is this parent's guard.
        let Some(other) = other else {
            return Ok((self_dir.lock(), None));
        };
        let other_dir = match other.dir() {
            Some(dir) => dir,
            None => {
                return Err(Error::with_message(
                    Errno::ENOTDIR,
                    "the target parent is not an overlay directory",
                ));
            }
        };
        // Stable object-identity order by `Arc` pointer address; the same-inode
        // case acquires the single `DIR` once (each parent exactly once).
        let self_addr = core::ptr::from_ref(self);
        let other_addr = Arc::as_ptr(other);
        if core::ptr::addr_eq(self_addr, other_addr) {
            return Ok((self_dir.lock(), None));
        }
        if self_addr < other_addr {
            let self_guard = self_dir.lock();
            let other_guard = other_dir.lock();
            Ok((self_guard, Some(other_guard)))
        } else {
            let other_guard = other_dir.lock();
            let self_guard = self_dir.lock();
            Ok((self_guard, Some(other_guard)))
        }
    }

    /// Returns the dentry-anchored path of the promoted upper real parent
    /// directory.
    ///
    /// The physical-operation target of every `Path`-routed recipe: after the
    /// mutating admission's promotion stage the facts carry an upper real
    /// object, and that object is always dentry-anchored by the inode
    /// invariant, so the checked `real_path()` accessor succeeds.
    /// `EROFS` surfaces when the facts carry no upper (a non-writable
    /// overlay); `EIO` propagates when the upper real object is not
    /// dentry-anchored or its anchor mount is no longer alive.
    pub(super) fn upper_parent_path(&self) -> Result<Path> {
        let facts = self.facts_snapshot();
        let upper = facts.upper().ok_or_else(|| {
            Error::with_message(Errno::EROFS, "the overlay object has no upper real parent")
        })?;
        upper.real_path()
    }

    /// Conservatively invalidates the stale projection of the affected
    /// `(parent, name)` pairs after a physical upper success whose semantic
    /// publication failed.
    ///
    /// The single shared private reconcile entry: a physical upper operation
    /// has committed but a
    /// BindingCache/barrier/index publication step failed, so the cached
    /// projection is stale; for each affected `(parent, name)` the
    /// mount-wide binding entry is invalidated (`BindingCache::invalidate`,
    /// keyed by `parent.key()`) and the parent's readdir index is marked
    /// `NeedsRebuild` (`invalidate_readdir_index`), so the next
    /// lookup/readdir re-derives from upper truth. The parents are plain
    /// `&OverlayInode` handles, so the one-parent `&self` recipes pass their
    /// own `(self, name)` pair directly and Arc-carrying recipes pass
    /// `(arc.as_ref(), name)` — no inlined two-step composition survives at
    /// any call site. Works for one- and two-parent operations and never
    /// claims a
    /// partial or stronger transaction. The mount upgrade is best-effort: on
    /// a dying mount there is no live cache to reconcile (no
    /// `.unwrap()`/`.expect()`).
    pub(super) fn invalidate_stale_cache(&self, affected: &[(&OverlayInode, &str)]) {
        let Ok(fs) = self.fs_arc() else {
            return;
        };
        for (parent, name) in affected {
            fs.bindings().invalidate(&parent.key(), name);
            parent.invalidate_readdir_index();
        }
    }
}
