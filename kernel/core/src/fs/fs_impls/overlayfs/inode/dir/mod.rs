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
//!   request and do not re-check permission. The `create_symlink` entry
//!   shares this contract and publishes the symlink atomically with its
//!   target — it replaced the retired two-step `create`-plus-`write_link`
//!   flow, so `create` rejects `SymLink` with `EINVAL` (the VFS routes
//!   symlink creation only through `create_symlink`).
//!
//! ## Structure
//!
//! | Submodule | Responsibility |
//! | --- | --- |
//! | `create` | create-object recipes (absent and over-whiteout branches) and the atomic symlink recipe |
//! | `link` | hard-link recipe |
//! | `remove` | shared unlink/rmdir recipe and whiteout-publish removal |
//! | `rename` | rename recipe |
//! | `whiteout` | shared whiteout cache and whiteout-publish mechanics |

use self::remove::RemoveKind;
use super::{
    AccessType, Lookup, NegativeLookup, OverlayInode, ReaddirIndex, permission::CopyUpOrigin,
};
use crate::{
    fs::{
        file::{InodeMode, InodeType, Permission},
        vfs::{
            inode::{Inode, MknodType, RenameMode},
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

type DirLockPair<'a, 'b> = (
    MutexGuard<'a, Option<ReaddirIndex>>,
    Option<MutexGuard<'b, Option<ReaddirIndex>>>,
);

impl OverlayInode {
    // Symlinks are never routed here: the VFS creates them only through
    // `create_symlink` (`create_symlink_impl` in `create.rs`).
    pub(super) fn create_impl(
        &self,
        self_dentry: &Dentry,
        name: &str,
        type_: InodeType,
        mode: InodeMode,
    ) -> Result<Arc<dyn Inode>> {
        if type_ == InodeType::SymLink {
            return_errno_with_message!(
                Errno::EINVAL,
                "overlay symlinks are created only through create_symlink"
            );
        }
        self.check_mutating_permission(
            CopyUpOrigin::Operation(self_dentry),
            Permission::MAY_WRITE,
        )?;
        let mut dir_guard = self.lock_dir_transaction();
        let projected: Arc<dyn Inode> =
            self.create_object(name, type_, mode, None, &mut dir_guard)?;
        Ok(projected)
    }

    pub(super) fn mknod_impl(
        &self,
        self_dentry: &Dentry,
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
        self.check_mutating_permission(
            CopyUpOrigin::Operation(self_dentry),
            Permission::MAY_WRITE,
        )?;
        let mut dir_guard = self.lock_dir_transaction();
        let object_type = crate::fs::fs_impls::overlayfs::inode::mknod_object_type(&type_);
        let projected: Arc<dyn Inode> =
            self.create_object(name, object_type, mode, Some(type_), &mut dir_guard)?;
        Ok(projected)
    }

    pub(super) fn is_stale_upper(
        &self,
        name: &str,
        fresh_lookup: &Lookup,
        current_index: &Option<ReaddirIndex>,
    ) -> bool {
        let Some(old_inode) = current_index
            .as_ref()
            .and_then(|idx| idx.visible_inode(name))
        else {
            return false;
        };
        if old_inode.upper.get().is_none() {
            return false;
        }
        match fresh_lookup {
            Lookup::Positive(inode) => !Arc::ptr_eq(inode, &old_inode),
            Lookup::Negative(negative) => *negative != NegativeLookup::HiddenByWhiteout,
        }
    }

    pub(super) fn link_impl(
        &self,
        self_dentry: &Dentry,
        old_dentry: &Dentry,
        name: &str,
    ) -> Result<()> {
        self.check_mutating_permission(
            CopyUpOrigin::Operation(self_dentry),
            Permission::MAY_WRITE,
        )?;
        let old_overlay =
            Arc::downcast::<OverlayInode>(old_dentry.inode().clone()).map_err(|_| {
                Error::with_message(Errno::EIO, "the link source is not an overlay inode")
            })?;
        // The VFS `link` syscall performs no source check of its own (VFS
        // gap), so these source-side checks are required; the parent's base
        // permission check remains authoritative.
        let source_metadata = old_overlay.metadata()?;
        let source_owned =
            super::permission::current_fsuid().is_some_and(|fsuid| fsuid == source_metadata.uid);
        // With no current task the source probe permits (no user credential
        // to check); in a user context these checks run in addition to the
        // parent's base permission check.
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
        let source_path = self.link_source(&old_overlay, old_dentry)?;
        let fs = self.fs_arc()?;
        let mut dir_guard = self.lock_dir_transaction();
        let target_lookup = fs.lookup(self, name)?;
        if matches!(target_lookup, Lookup::Positive(_)) {
            return Err(Error::new(Errno::ESTALE));
        }
        let target_is_whiteout = matches!(
            target_lookup,
            Lookup::Negative(NegativeLookup::HiddenByWhiteout)
        );
        // A source with lower fallback makes this parent impure: persist the
        // impure marker to the upper parent before either physical-link
        // branch (before committing the link).
        if !old_overlay.lowers.is_empty() {
            let upper_parent_path = self.upper_parent_path()?;
            OverlayInode::set_impure_marker(
                upper_parent_path.inode(),
                upper_parent_path.dentry(),
                fs.policy().xattr_prefix(),
            )?;
        }
        if target_is_whiteout {
            self.link_over_whiteout(name, &source_path)?;
        } else {
            self.upper_parent_path()?.link(&source_path, name)?;
        }
        self.readdir_index_insert(
            name,
            old_overlay.clone(),
            old_overlay.type_(),
            &mut dir_guard,
        );
        Ok(())
    }

    pub(super) fn unlink_impl(&self, child_dentry: &Dentry, name: String) -> Result<()> {
        // The admission promotes the parent directory itself, sourcing its
        // publication coordinate from the removed entry's parent dentry; the
        // root case is structurally unreachable and falls back to the
        // anchor origin.
        let parent_dentry = child_dentry.parent();
        let origin = match &parent_dentry {
            Some(parent) => CopyUpOrigin::Operation(parent),
            None => CopyUpOrigin::Anchor,
        };
        self.check_mutating_permission(origin, Permission::MAY_WRITE)?;
        let mut dir_guard = self.lock_dir_transaction();
        self.remove_target(&name, RemoveKind::Unlink, &mut dir_guard)
    }

    pub(super) fn rmdir_impl(&self, child_dentry: &Dentry, name: String) -> Result<()> {
        let parent_dentry = child_dentry.parent();
        let origin = match &parent_dentry {
            Some(parent) => CopyUpOrigin::Operation(parent),
            None => CopyUpOrigin::Anchor,
        };
        self.check_mutating_permission(origin, Permission::MAY_WRITE)?;
        let mut dir_guard = self.lock_dir_transaction();
        self.remove_target(&name, RemoveKind::Rmdir, &mut dir_guard)
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
        let parent_dentry = old_child_dentry.parent();
        let self_origin = match &parent_dentry {
            Some(parent) => CopyUpOrigin::Operation(parent),
            None => CopyUpOrigin::Anchor,
        };
        self.check_mutating_permission(self_origin, Permission::MAY_WRITE)?;
        target_overlay.check_mutating_permission(
            CopyUpOrigin::Operation(new_dir_dentry),
            Permission::MAY_WRITE,
        )?;
        source_overlay.copy_up_at(old_child_dentry)?;
        if !core::ptr::addr_eq(core::ptr::from_ref(self), Arc::as_ptr(&target_overlay)) {
            self.cross_device_gate(&source_overlay)?;
        }
        let (mut source_guard, mut target_guard) =
            self.lock_parent_dir_transactions(Some(&target_overlay))?;
        self.rename_upper(
            &old_name,
            &source_overlay,
            &target_overlay,
            new_name,
            replaced_inode.as_ref(),
            mode,
            rename::RenameLocks {
                self_index: &mut source_guard,
                target_index: target_guard.as_deref_mut(),
            },
        )
    }
}

impl OverlayInode {
    /// Non-directories still carry this lock as a plain serialization token;
    /// the `ReaddirIndex` payload is meaningful only for directories.
    fn lock_dir_transaction(&self) -> MutexGuard<'_, Option<ReaddirIndex>> {
        self.lock.lock()
    }

    /// Acquires both parent guards in a fixed `Arc::as_ptr` address order,
    /// each parent exactly once, so two-lock acquisition cannot deadlock.
    fn lock_parent_dir_transactions<'a, 'b>(
        &'a self,
        other: Option<&'b Arc<OverlayInode>>,
    ) -> Result<DirLockPair<'a, 'b>> {
        if self.lock.lock().is_none() {
            return Err(Error::with_message(
                Errno::ENOTDIR,
                "the source parent is not an overlay directory",
            ));
        }
        let Some(other) = other else {
            return Ok((self.lock.lock(), None));
        };
        if other.lock.lock().is_none() {
            return Err(Error::with_message(
                Errno::ENOTDIR,
                "the target parent is not an overlay directory",
            ));
        }
        let self_addr = core::ptr::from_ref(self);
        let other_addr = Arc::as_ptr(other);
        if core::ptr::addr_eq(self_addr, other_addr) {
            return Ok((self.lock.lock(), None));
        }
        if self_addr < other_addr {
            let self_guard = self.lock.lock();
            let other_guard = other.lock.lock();
            Ok((self_guard, Some(other_guard)))
        } else {
            let other_guard = other.lock.lock();
            let self_guard = self.lock.lock();
            Ok((self_guard, Some(other_guard)))
        }
    }
}
