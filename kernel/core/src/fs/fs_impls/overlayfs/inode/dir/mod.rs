// SPDX-License-Identifier: MPL-2.0

//! The overlayfs namespace-mutation and whiteout subsystem.
//!
//! This module hosts the `Inode`-trait entries for directory name-space
//! mutations: create (which also serves mkdir), create_symlink, mknod, link,
//! unlink, rmdir, and rename. Each entry resolves a fresh projection of the
//! target name under the parent directory transaction lock and delegates the
//! actual mutation to a per-directory recipe.
//!
//! Key concepts:
//! - **lookup**: the overlay-visible answer for a `(parent, name)` pair — a
//!   positive inode, or a negative reason (`Absent`, `HiddenByWhiteout`).
//! - **parent directory transaction**: the per-directory `Mutex` guard that
//!   serializes mutation recipes for one parent.
//! - **whiteout**: an upper-layer visibility barrier published when a
//!   lower-backed name is removed; the `whiteout` submodule owns its cache and
//!   publish mechanics.
//! - **entry admission contract**: the parent-lock-taking entries run
//!   `check_mutating_permission(...)` (including any required copy-up
//!   promotion) before acquiring the parent directory transaction lock;
//!   rename additionally pre-promotes the source before taking either parent
//!   lock. The recipes therefore assume the caller already admitted the
//!   request and do not re-check permission. All three create-family entries
//!   (`create`, `create_symlink`, `mknod`) flow through one `CreateOp` into the
//!   single `create_object` dispatch; the `CreateOp::general` entry rejects
//!   `SymLink` with `EINVAL`, because the VFS routes symlink creation only
//!   through `create_symlink`, which publishes the symlink atomically with its
//!   target.
//!
//! ## Structure
//!
//! | Submodule | Responsibility |
//! | --- | --- |
//! | `create` | the single `CreateOp` create-object dispatch (absent and over-whiteout branches, atomic symlink) |
//! | `link` | hard-link recipe |
//! | `remove` | shared unlink/rmdir recipe and whiteout-publish removal |
//! | `rename` | rename recipe |
//! | `whiteout` | shared whiteout cache and whiteout-publish mechanics |

use self::remove::RemoveKind;
use super::{AccessType, CreateOp, Lookup, NegativeLookup, OverlayInode};
use crate::{
    fs::{
        file::{InodeType, Permission},
        vfs::{
            inode::{Inode, RenameMode},
            path::Dentry,
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

impl OverlayInode {
    /// The single create-family entry; `CreateOp` encodes the VFS entry that produced it.
    pub(super) fn create_impl(
        &self,
        self_dentry: &Dentry,
        name: &str,
        op: &CreateOp<'_>,
    ) -> Result<Arc<dyn Inode>> {
        self.check_mutating_permission(self_dentry, Permission::MAY_WRITE)?;
        let mut dir_guard = self.lock();
        let projected: Arc<dyn Inode> = self.create_object(name, op, &mut dir_guard)?;
        Ok(projected)
    }

    pub(super) fn link_impl(
        &self,
        self_dentry: &Dentry,
        old_dentry: &Dentry,
        name: &str,
    ) -> Result<()> {
        self.check_mutating_permission(self_dentry, Permission::MAY_WRITE)?;
        let old_overlay =
            Arc::downcast::<OverlayInode>(old_dentry.inode().clone()).map_err(|_| {
                Error::with_message(Errno::EIO, "the link source is not an overlay inode")
            })?;
        // The VFS `link` syscall does no source check, so these source-side checks are required.
        let source_metadata = old_overlay.metadata()?;
        let source_owned =
            super::permission::current_fsuid().is_some_and(|fsuid| fsuid == source_metadata.uid);
        // With no current task the probe permits; otherwise these checks add to the parent's.
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
                && !super::permission::current_task_has_capability(CapSet::FOWNER)
            {
                return Err(Error::with_message(
                    Errno::EPERM,
                    "the link source is set-id and the caller lacks CAP_FOWNER",
                ));
            }
        }
        let source_dentry = self.link_source(&old_overlay, old_dentry)?;
        let fs = self.fs_arc();
        let mut dir_guard = self.lock();
        let target_lookup = fs.lookup(self, name)?;
        if matches!(target_lookup, Lookup::Positive(_)) {
            return Err(Error::new(Errno::ESTALE));
        }
        let target_is_whiteout = matches!(
            target_lookup,
            Lookup::Negative(NegativeLookup::HiddenByWhiteout)
        );
        // A lower-backed source makes the parent impure; persist the marker before the link.
        if !old_overlay.lowers.is_empty() {
            let upper_parent = self.upper_parent_dentry()?;
            if !fs.policy().can_store_private_xattr() {
                return Err(Error::with_message(
                    Errno::EOPNOTSUPP,
                    "the upper filesystem cannot store the impure marker required for a link",
                ));
            }
            OverlayInode::set_impure_marker(
                upper_parent.inode(),
                upper_parent,
                fs.policy().xattr_namespace(),
            )?;
        }
        if target_is_whiteout {
            self.link_over_whiteout(name, &source_dentry)?;
        } else {
            self.upper_parent_dentry()?
                .as_dir_dentry_or_err()?
                .link(&source_dentry, name)?;
        }
        // Origin-preserved iff the linked source retains a lower stack; that is this `is_impure`.
        self.readdir_cache_insert(
            name,
            old_overlay.type_(),
            old_overlay.ino(),
            !old_overlay.lowers.is_empty(),
            &mut dir_guard,
        );
        Ok(())
    }

    pub(super) fn unlink_impl(&self, child_dentry: &Dentry, name: String) -> Result<()> {
        // This check promotes the parent, sourcing its target from the removed entry's parent.
        let parent_dentry = child_dentry.parent().ok_or_else(|| {
            Error::with_message(Errno::EIO, "the unlinked child has no parent dentry")
        })?;
        self.check_mutating_permission(&parent_dentry, Permission::MAY_WRITE)?;
        let mut dir_guard = self.lock();
        self.remove_target(child_dentry, &name, RemoveKind::Unlink, &mut dir_guard)
    }

    pub(super) fn rmdir_impl(&self, child_dentry: &Dentry, name: String) -> Result<()> {
        // VFS names the child through its parent, so `None` is only a fail-closed safety net.
        let parent_dentry = child_dentry.parent().ok_or_else(|| {
            Error::with_message(Errno::EIO, "the removed directory has no parent dentry")
        })?;
        self.check_mutating_permission(&parent_dentry, Permission::MAY_WRITE)?;
        let mut dir_guard = self.lock();
        self.remove_target(child_dentry, &name, RemoveKind::Rmdir, &mut dir_guard)
    }

    pub(super) fn rename_impl(
        &self,
        old_child_dentry: &Dentry,
        new_dir_dentry: &Dentry,
        new_name: &str,
        target_dentry: Option<&Dentry>,
        mode: RenameMode,
    ) -> Result<()> {
        let old_name = old_child_dentry.name();
        let source_overlay = Arc::downcast::<OverlayInode>(old_child_dentry.inode().clone())
            .map_err(|_| {
                Error::with_message(Errno::EIO, "the rename source is not an overlay inode")
            })?;
        let target_overlay = Arc::downcast::<OverlayInode>(new_dir_dentry.inode().clone())
            .map_err(|_| {
                Error::with_message(Errno::EIO, "the rename target is not an overlay inode")
            })?;
        let replaced_inode = target_dentry.map(|d| d.inode().clone());
        // VFS names the child through its parent, so `None` is only a fail-closed safety net.
        let parent_dentry = old_child_dentry.parent().ok_or_else(|| {
            Error::with_message(Errno::EIO, "the renamed child has no parent dentry")
        })?;
        self.check_mutating_permission(&parent_dentry, Permission::MAY_WRITE)?;
        target_overlay.check_mutating_permission(new_dir_dentry, Permission::MAY_WRITE)?;
        source_overlay.copy_up_at(old_child_dentry)?;
        if !core::ptr::addr_eq(core::ptr::from_ref(self), Arc::as_ptr(&target_overlay)) {
            self.cross_device_gate(&source_overlay)?;
        }
        let locks = rename::RenameLocks::acquire(self, &target_overlay)?;
        self.rename_upper(
            &old_name,
            &source_overlay,
            &target_overlay,
            new_name,
            replaced_inode.as_ref(),
            mode,
            locks,
        )
    }
}
