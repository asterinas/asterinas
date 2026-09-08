// SPDX-License-Identifier: MPL-2.0
#![short_vis_path::add(overlayfs)]

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
//! publication coordinate from [`CopyUpOrigin`].

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AccessType {
    ReadOnly,
    Mutating,
}

/// The context a mutating admission uses to source the copy-up publication
/// coordinate. `Operation` carries the operation's overlay dentry and reads
/// (parent, name) from it at entry — this is also the form of rename's
/// new-parent admission. `Anchor` serves the dentry-less entries (fallocate,
/// and the structurally unreachable parentless fallback of
/// unlink/rmdir/rename): the parent is re-resolved at the object's anchor
/// path and the name is the visible-source real dentry's name, failing
/// closed on divergence.
pub(super) enum CopyUpOrigin<'a> {
    Operation(&'a Dentry),
    Anchor,
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

/// Kernel contexts fail open (no user to gate); a user context whose
/// thread-local user namespace is absent fails closed.
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

/// The fsgid disjunct closes the owner-chgrp exemption gap: an owner whose
/// fsgid is the target gid must not be denied because the supplementary set
/// omits it.
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
    /// Verdicts are never cached.
    ///
    /// The no-promotion admission: the local gate plus the real re-check.
    /// Every mutating flow — including the copy-up promotion — runs only
    /// through [`Self::check_mutating_permission`]; this entry never
    /// promotes.
    pub(super) fn check_permission(&self, access: AccessType, perm: Permission) -> Result<()> {
        self.check_local_permission(access, perm)?;
        if !self.fs_arc()?.policy().is_default_permissions() {
            self.check_real_permission(perm)?;
        }
        Ok(())
    }

    /// The mutating admission: the local gate (`Mutating`: EROFS + projected
    /// DAC), the copy-up promotion sourced from `origin`, then the real
    /// re-check — the same stage order as [`Self::check_permission`].
    pub(super) fn check_mutating_permission(
        &self,
        origin: CopyUpOrigin<'_>,
        perm: Permission,
    ) -> Result<()> {
        self.check_local_permission(AccessType::Mutating, perm)?;
        match origin {
            CopyUpOrigin::Operation(dentry) => self.copy_up_at(dentry)?,
            CopyUpOrigin::Anchor => self.copy_up_at_anchor()?,
        }
        if !self.fs_arc()?.policy().is_default_permissions() {
            self.check_real_permission(perm)?;
        }
        Ok(())
    }

    fn check_local_permission(&self, access: AccessType, mut perm: Permission) -> Result<()> {
        if access == AccessType::Mutating && self.fs_arc()?.policy().is_effective_read_only() {
            return_errno_with_message!(Errno::EROFS, "the overlay mount is read-only");
        }

        // TODO(VFS gap): this block mirrors the VFS default
        // `Inode::check_permission`; extract a shared `check_mode_dac` once
        // the VFS provides one.
        let Some(creds) = with_current_posix_thread(|_, posix_thread| posix_thread.credentials())
        else {
            return Ok(());
        };
        let metadata = self.metadata()?;
        let mode = metadata.mode;

        // With `DAC_OVERRIDE`, read/write DACs are always overridable; exec
        // only when at least one execute bit is set.
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

        // No protected-state check is needed here: protected (private-prefix)
        // names are already routed and gated by the xattr classification in
        // `inode/xattr.rs` (`classify` / `XattrClass`).
        Ok(())
    }

    /// The explicit re-check is a benign double evaluation for xattr ops
    /// that already self-evaluate under the caller's current credentials.
    fn check_real_permission(&self, perm: Permission) -> Result<()> {
        let real = self.select_real_inode();
        real.check_permission(perm)
    }
}
