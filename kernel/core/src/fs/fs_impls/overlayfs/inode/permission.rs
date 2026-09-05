// SPDX-License-Identifier: MPL-2.0

//! The two-stage permission admission pipeline.
//!
//! This module hosts the two admission entries
//! ([`OverlayInode::check_permission`] for read-only requests,
//! [`OverlayInode::check_mutating_permission`] for mutating requests) plus
//! the two stage helpers, and the shared current-credential probes used by
//! the metadata entries.
//!
//! # Stages
//!
//! | Stage | Function |
//! |---|---|
//! | Local | [`OverlayInode::check_local_permission`] (EROFS gate + the overlay-local DAC check). |
//! | Real | [`OverlayInode::check_real_permission`] (explicit real re-check). |
//!
//! The read-only `Inode::check_permission` forwarder calls the read-only
//! entry with `AccessType::ReadOnly`, never promoting. The mutating entry
//! inserts the copy-up promotion between the two stages, sourcing the
//! publication coordinate from the caller's overlay dentry.

#![short_vis_path::add(overlayfs)]

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AccessType {
    ReadOnly,
    Mutating,
}

use super::OverlayInode;
use crate::{
    fs::{
        file::Permission,
        fs_impls::overlayfs::with_current_posix_thread,
        vfs::{inode::Inode, path::Dentry},
    },
    prelude::*,
    process::{Gid, Uid, credentials::capabilities::CapSet},
    security::lsm::hooks as lsm_hooks,
};

/// Kernel contexts fail open; a user context with no thread-local user namespace fails closed.
pub(super) fn current_task_has_capability(cap: CapSet) -> bool {
    let Some(has_cap) = with_current_posix_thread(|task, posix_thread| {
        task.as_thread_local().is_some_and(|thread_local| {
            let user_ns = thread_local.borrow_user_ns();
            lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
                user_ns.as_ref(),
                posix_thread,
                cap,
            ))
            .is_ok()
        })
    }) else {
        return true;
    };
    has_cap
}

pub(super) fn current_fsuid() -> Option<Uid> {
    let fsuid = with_current_posix_thread(|_, posix_thread| posix_thread.credentials().fsuid())?;
    Some(fsuid)
}

/// The fsgid disjunct closes the owner-chgrp gap when the supplementary set omits the target gid.
pub(super) fn current_in_group(gid: Gid) -> bool {
    let Some(in_group) = with_current_posix_thread(|_, posix_thread| {
        let credentials = posix_thread.credentials();
        gid == credentials.fsgid() || credentials.groups().contains(&gid)
    }) else {
        return false;
    };
    in_group
}

impl OverlayInode {
    /// The no-copy-up check: local gate plus real re-check; verdicts are never cached.
    pub(super) fn check_permission(&self, access: AccessType, perm: Permission) -> Result<()> {
        self.check_local_permission(access, perm)?;
        if !self.fs_arc().policy().is_default_permissions() {
            self.check_real_permission(perm)?;
        }
        Ok(())
    }

    /// The mutating check: local gate, copy-up, then the real re-check.
    pub(super) fn check_mutating_permission(
        &self,
        self_dentry: &Dentry,
        perm: Permission,
    ) -> Result<()> {
        self.check_local_permission(AccessType::Mutating, perm)?;
        self.copy_up_at(self_dentry)?;
        if !self.fs_arc().policy().is_default_permissions() {
            self.check_real_permission(perm)?;
        }
        Ok(())
    }

    fn check_local_permission(&self, access: AccessType, mut perm: Permission) -> Result<()> {
        if access == AccessType::Mutating && self.fs_arc().policy().is_effective_read_only() {
            return_errno_with_message!(Errno::EROFS, "the overlay mount is read-only");
        }

        // TODO(VFS gap): mirrors VFS `Inode::check_permission`; extract once VFS shares one.
        let Some(creds) = with_current_posix_thread(|_, posix_thread| posix_thread.credentials())
        else {
            return Ok(());
        };
        let metadata = self.metadata()?;
        let mode = metadata.mode;

        // `DAC_OVERRIDE` always allows read/write; exec needs at least one execute bit.
        let has_dac_override = current_task_has_capability(CapSet::DAC_OVERRIDE);
        if has_dac_override {
            perm -= Permission::MAY_READ | Permission::MAY_WRITE;
            if perm.may_exec() {
                if mode.is_owner_executable()
                    || mode.is_group_executable()
                    || mode.is_other_executable()
                {
                    perm -= Permission::MAY_EXEC;
                } else {
                    return_errno_with_message!(
                        Errno::EACCES,
                        "root execute permission denied: no execute bits set"
                    );
                }
            }
        }

        if metadata.uid == creds.fsuid() {
            if (perm.may_read() && !mode.is_owner_readable())
                || (perm.may_write() && !mode.is_owner_writable())
                || (perm.may_exec() && !mode.is_owner_executable())
            {
                return_errno_with_message!(Errno::EACCES, "owner permission check failed");
            }
        } else if metadata.gid == creds.fsgid() {
            if (perm.may_read() && !mode.is_group_readable())
                || (perm.may_write() && !mode.is_group_writable())
                || (perm.may_exec() && !mode.is_group_executable())
            {
                return_errno_with_message!(Errno::EACCES, "group permission check failed");
            }
        } else if (perm.may_read() && !mode.is_other_readable())
            || (perm.may_write() && !mode.is_other_writable())
            || (perm.may_exec() && !mode.is_other_executable())
        {
            return_errno_with_message!(Errno::EACCES, "other permission check failed");
        }

        // No protected-state check here: protected names are gated by xattr classification.
        Ok(())
    }

    /// The re-check is a benign double evaluation for xattr ops that already self-evaluate.
    fn check_real_permission(&self, perm: Permission) -> Result<()> {
        let real = self.visible_source().real_inode();
        real.check_permission(perm)
    }
}
