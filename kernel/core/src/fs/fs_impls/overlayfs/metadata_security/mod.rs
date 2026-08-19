// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(overlayfs)]
//! The module root and entry surface of the metadata, permission, and xattr
//! policy subsystem: it supplies the shared helper
//! ([`OverlayInode::delegate_to_real`]) across the three layers it gathers —
//! the two-stage permission pipeline, metadata setters, and xattr-name
//! classification and entry bounds. The overlay xattr policy is owned by
//! `OverlayFs` as a `pub(super)` field.
//!

use super::{inode::OverlayInode, with_current_posix_thread};
use crate::{
    fs::vfs::inode::Inode,
    prelude::*,
    process::{Gid, Uid, credentials::capabilities::CapSet},
    security::lsm::hooks as lsm_hooks,
};

mod metadata;
mod permission;
pub(super) mod xattr;

/// Returns whether the current task holds `cap` in its user namespace
/// (the single shared capability probe of this module). Kernel contexts
/// fail open (`true` — no user to gate); a user context whose
/// thread-local (and thus user namespace) is absent fails closed
/// (`false` — no namespace to scope against).
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
        // Kernel contexts fail open: there is no user to gate.
        return true;
    };
    has_cap
}

/// Returns the current task's filesystem UID (`None` in a kernel context
/// — no task / no posix thread).
///
/// Callers treat `None` as "not the owner" (the shared kernel-context
/// default applied via `is_some_and`). Consumed by the ownership-sensitive
/// metadata logic and the source-side link admission.
pub(super) fn current_fsuid() -> Option<Uid> {
    let fsuid = with_current_posix_thread(|_, posix_thread| posix_thread.credentials().fsuid())?;
    Some(fsuid)
}

/// Returns whether `gid` equals the current task's filesystem group ID or
/// is in its supplementary group set (kernel contexts report `false`).
///
/// The fsgid disjunct closes the owner-chgrp exemption gap: without it, an owner
/// whose filesystem group ID is the target gid but whose supplementary
/// set omits it would be denied the owner-chgrp exemption.
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
    /// Runs `operation_fn` directly against the current real authority.
    ///
    /// Precondition: the permission stage has already admitted the operation
    /// (or the entry is a pure read delegation).
    fn delegate_to_real<T>(
        &self,
        operation_fn: impl FnOnce(&Arc<dyn Inode>) -> Result<T>,
    ) -> Result<T> {
        // The generic `T` is deliberate:
        // delegated operations return heterogeneous types,
        // so one helper avoids dedicated per-kind carriers.
        // Metadata setters whose real ops do not self-evaluate
        // additionally ran the explicit real check first.
        let real = self.select_real_inode();
        operation_fn(&real)
    }
}
