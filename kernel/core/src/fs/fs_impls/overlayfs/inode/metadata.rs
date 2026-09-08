// SPDX-License-Identifier: MPL-2.0
#![short_vis_path::add(overlayfs)]

//! The metadata-mutation entries.
//!
//! This module hosts the six metadata setters: `set_mode`/`set_owner`/
//! `set_group` (chmod/chown) and `set_atime`/`set_mtime`/`set_ctime`
//! (utimes). Every entry admits through
//! [`OverlayInode::check_mutating_permission`] — the local permission check,
//! copy-up promotion sourced from the entry's own dentry, and the
//! real-handle re-check (unless `default_permissions`) — then forwards to
//! the real authority via [`OverlayInode::delegate_to_real`].
//!
//! # Ownership gate
//!
//! The pipeline checks mode DAC only: neither the syscall layer nor the
//! pipeline performs an owner/`CAP_FOWNER`/`CAP_CHOWN` pre-check. The
//! ownership-sensitive setters (`set_mode`, `set_owner`, `set_group`)
//! therefore run a local **ownership/capability gate** before the uniform
//! admission, and admit with `Permission::empty()`. Ownership/capability
//! alone is then authoritative, so the chmod-000-then-chmod-644 owner idiom
//! never fails with `EACCES`. This is a deliberate deviation from an
//! admission that only passes `MAY_WRITE`.
//!
//! The time setters follow the utimensat disjunction instead: `MAY_WRITE`,
//! or an owner/`CAP_FOWNER` fallback that re-runs admission with
//! `Permission::empty()`; failures are best-effort silent no-ops.

use core::time::Duration;

use super::{
    OverlayInode,
    permission::{CopyUpOrigin, current_fsuid, current_in_group, current_task_has_capability},
};
use crate::{
    fs::{
        file::{InodeMode, Permission},
        vfs::{inode::Inode, path::Dentry},
    },
    prelude::*,
    process::{Gid, Uid, credentials::capabilities::CapSet},
};

impl OverlayInode {
    pub(super) fn set_mode_impl(&self, self_dentry: &Dentry, mode: InodeMode) -> Result<()> {
        let metadata = self.metadata()?;
        let is_owner = current_fsuid().is_some_and(|fsuid| fsuid == metadata.uid);
        let has_cap = current_task_has_capability(CapSet::FOWNER);
        if !is_owner && !has_cap {
            return Err(Error::with_message(
                Errno::EPERM,
                "the caller is not the file owner and lacks CAP_FOWNER",
            ));
        }
        let has_fsetid = current_task_has_capability(CapSet::FSETID);
        let mut mode = mode;
        if !is_owner && !has_fsetid {
            mode.remove(InodeMode::S_ISUID | InodeMode::S_ISGID);
        }
        self.check_mutating_permission(CopyUpOrigin::Operation(self_dentry), Permission::empty())?;
        self.delegate_to_real(|real, d| real.set_mode(d, mode))
    }

    pub(super) fn set_owner_impl(&self, self_dentry: &Dentry, uid: Uid) -> Result<()> {
        let metadata = self.metadata()?;
        if uid != metadata.uid && !current_task_has_capability(CapSet::CHOWN) {
            return Err(Error::with_message(
                Errno::EPERM,
                "the caller lacks CAP_CHOWN for an ownership change",
            ));
        }
        self.check_mutating_permission(CopyUpOrigin::Operation(self_dentry), Permission::empty())?;
        self.delegate_to_real(|real, d| real.set_owner(d, uid))
    }

    pub(super) fn set_group_impl(&self, self_dentry: &Dentry, gid: Gid) -> Result<()> {
        let metadata = self.metadata()?;
        if gid != metadata.gid {
            let is_owner = current_fsuid().is_some_and(|fsuid| fsuid == metadata.uid);
            let has_cap = current_task_has_capability(CapSet::CHOWN);
            // The owner-chgrp exemption: the owner may change the group to
            // one of its own supplementary groups.
            let in_own_group = current_in_group(gid);
            if !has_cap && !(is_owner && in_own_group) {
                return Err(Error::with_message(
                    Errno::EPERM,
                    "the caller lacks CAP_CHOWN for a group change",
                ));
            }
        }
        self.check_mutating_permission(CopyUpOrigin::Operation(self_dentry), Permission::empty())?;
        self.delegate_to_real(|real, d| real.set_group(d, gid))
    }

    pub(super) fn set_atime_impl(&self, self_dentry: &Dentry, time: Duration) {
        self.best_effort_time_set(self_dentry, |real, d| real.set_atime(d, time));
    }

    pub(super) fn set_mtime_impl(&self, self_dentry: &Dentry, time: Duration) {
        self.best_effort_time_set(self_dentry, |real, d| real.set_mtime(d, time));
    }

    pub(super) fn set_ctime_impl(&self, self_dentry: &Dentry, time: Duration) {
        self.best_effort_time_set(self_dentry, |real, d| real.set_ctime(d, time));
    }
}

impl OverlayInode {
    fn best_effort_time_set(
        &self,
        self_dentry: &Dentry,
        operation_fn: impl FnOnce(&Arc<dyn Inode>, &Dentry),
    ) {
        let Some(metadata) = self.metadata().ok() else {
            return;
        };
        let is_owner = current_fsuid().is_some_and(|fsuid| fsuid == metadata.uid);
        let has_cap = current_task_has_capability(CapSet::FOWNER);
        if self
            .check_mutating_permission(CopyUpOrigin::Operation(self_dentry), Permission::MAY_WRITE)
            .is_err()
        {
            if !is_owner && !has_cap {
                return;
            }
            if self
                .check_mutating_permission(
                    CopyUpOrigin::Operation(self_dentry),
                    Permission::empty(),
                )
                .is_err()
            {
                return;
            }
        }
        let _ = self.delegate_to_real(|real, d| {
            operation_fn(real, d);
            Ok(())
        });
    }
}
