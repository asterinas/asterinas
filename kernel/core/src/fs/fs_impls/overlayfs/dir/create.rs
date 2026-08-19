// SPDX-License-Identifier: MPL-2.0

//! The create-object recipes.
//!
//! This module hosts [`OverlayInode::create_object`] (dispatcher),
//! [`OverlayInode::create_upper_only`], and
//! [`OverlayInode::create_over_whiteout`] (over-whiteout/opaque branch).
//!
use crate::{
    fs::{
        file::{InodeMode, InodeType},
        fs_impls::overlayfs::{
            inode::{ObjectFacts, OverlayInode},
            projection::{Binding, NegativeBinding, PositiveKind},
            workdir::WorkdirTempRequest,
        },
        vfs::inode::{MknodType, RenameMode},
    },
    prelude::*,
};

impl OverlayInode {
    /// Dispatches one create-family request (create/mkdir/mknod/symlink)
    /// from the fresh `(parent, name)` projection under the parent
    /// directory transaction lock.
    ///
    /// Decides on current `BindingCache` evidence, never the stale VFS
    /// negative dentry that triggered the call.
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

    /// Creates a genuinely absent object directly in the upper parent — no
    /// workdir, no whiteout.
    ///
    /// Precondition: the caller holds this parent's directory transaction
    /// lock and has already run
    /// `check_permission(AccessType::Mutating, Permission::MAY_WRITE)`.
    fn create_upper_only(
        &self,
        name: &str,
        type_: InodeType,
        mode: InodeMode,
        mknod_type: Option<MknodType>,
    ) -> Result<Arc<OverlayInode>> {
        let fs = self.fs_arc()?;
        let upper_parent_path = self.upper_parent_path()?;
        let object_type = mknod_type
            .as_ref()
            .map(crate::fs::fs_impls::overlayfs::workdir::mknod_object_type)
            .unwrap_or(type_);
        let new_upper_path = match mknod_type {
            Some(mknod) => upper_parent_path.mknod(name, mode, mknod)?,
            None => upper_parent_path.new_fs_child(name, type_, mode)?,
        };
        let upper_layer = fs.layer_stack.upper_layer()?;
        let new_facts = ObjectFacts::try_new(
            PositiveKind::Single,
            Some(upper_layer.child_real_object(&new_upper_path)),
            Vec::new(),
        )
        .ok_or_else(|| {
            Error::with_message(
                Errno::EIO,
                "the new upper object facts are not constructible",
            )
        })?;
        let inode = fs.project_inode(&new_facts);
        self.publish_positive_binding(&fs, name, inode.clone(), object_type);
        Ok(inode)
    }

    /// Replaces a whiteout-hidden name with a completely prepared private
    /// workdir temp, then publishes it.
    ///
    /// A failure before the atomic upper commit best-effort-cleans the temp;
    /// a failure after the commit reconciles the affected `(parent, name)`
    /// projection as a unit via the shared
    /// [`OverlayInode::invalidate_stale_cache`] entry.
    fn create_over_whiteout(
        &self,
        name: &str,
        type_: InodeType,
        mode: InodeMode,
        mknod_type: Option<MknodType>,
    ) -> Result<Arc<OverlayInode>> {
        let fs = self.fs_arc()?;
        let upper_parent_path = self.upper_parent_path()?;
        let object_type = mknod_type
            .as_ref()
            .map(crate::fs::fs_impls::overlayfs::workdir::mknod_object_type)
            .unwrap_or(type_);
        let temp = match &mknod_type {
            Some(node) => fs.create_workdir_temp(name, WorkdirTempRequest::Mknod { mode, node })?,
            None => {
                fs.create_workdir_temp(name, WorkdirTempRequest::Create { kind: type_, mode })?
            }
        };
        let temp_kind = temp.kind();
        let (temp_name, temp) = temp.into_parts();
        let workdir_path = self.workdir_root_path()?;
        let mut committed = false;
        let result: Result<Arc<OverlayInode>> = (|| {
            if object_type == InodeType::Dir {
                // Opaque branch: the opaque record is part of the
                // replacement directory's complete publication; the
                // marker write is gated by the private-xattr capability
                // and runs on the temp before the atomic swap — the
                // whiteout is never deleted first.
                fs.set_opaque_marker(
                    temp.inode(),
                    "the upper filesystem cannot store the opaque marker \
                     required for a directory over a whiteout",
                )?;
            }
            if object_type.is_directory() {
                workdir_path.rename(&temp_name, &upper_parent_path, name, RenameMode::Exchange)?;
                committed = true;
                workdir_path.unlink(&temp_name)?;
            } else {
                workdir_path.rename(&temp_name, &upper_parent_path, name, RenameMode::Replace)?;
                committed = true;
            }
            // Semantic publication: the temp handle is the published object at
            // `(upper_parent_path, name)` (inode identity is stable across the
            // rename).
            let upper_layer = fs.layer_stack.upper_layer()?;
            let new_facts = ObjectFacts::try_new(
                PositiveKind::Single,
                Some(upper_layer.child_real_object(&temp)),
                Vec::new(),
            )
            .ok_or_else(|| {
                Error::with_message(
                    Errno::EIO,
                    "the new upper object facts are not constructible",
                )
            })?;
            let inode = fs.project_inode(&new_facts);
            self.publish_positive_binding(&fs, name, inode.clone(), object_type);
            Ok(inode)
        })();
        match result {
            Ok(inode) => Ok(inode),
            Err(err) => {
                if committed {
                    self.invalidate_stale_cache(&[(self, name)]);
                } else {
                    // Pre-commit failure (pre-publication arm): best-effort
                    // kind-aware temp cleanup; residue is a known cleanup
                    // debt, never a visible source.
                    let _ = fs.cleanup_workdir_temp(&temp_name, temp_kind);
                }
                Err(err)
            }
        }
    }
}
