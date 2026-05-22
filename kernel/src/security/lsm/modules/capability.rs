// SPDX-License-Identifier: MPL-2.0

use super::super::{
    AlienAccessContext, CapableContext, InodeDacOverrideContext, LsmFlags, LsmModule,
    hooks::{LsmAlienAccessHook, LsmCapabilityHook, LsmInodeHook},
};
use crate::{
    fs::file::Permission,
    prelude::*,
    process::{credentials::capabilities::CapSet, posix_thread::alien_access::CredsSource},
};

pub(super) static CAPABILITY_LSM: CapabilityLsm = CapabilityLsm;

/// Capability-based authorization checks for built-in kernel operations.
pub(super) struct CapabilityLsm;

impl LsmModule for CapabilityLsm {
    fn name(&self) -> &'static str {
        "capability"
    }

    fn flags(&self) -> LsmFlags {
        LsmFlags::empty()
    }
}

impl LsmCapabilityHook for CapabilityLsm {
    fn on_capable(&self, context: &CapableContext) -> Result<()> {
        let _ = (context.user_namespace(), context.reason());
        if context
            .posix_thread()
            .credentials()
            .effective_capset()
            .contains(context.capability())
        {
            return Ok(());
        }

        return_errno_with_message!(
            Errno::EPERM,
            "the thread does not have the required capability"
        );
    }
}

impl LsmAlienAccessHook for CapabilityLsm {
    fn on_alien_access(&self, context: &AlienAccessContext) -> Result<()> {
        let accessor_cred = context.accessor().credentials();
        let (caller_uid, caller_gid) = match context.mode().creds() {
            CredsSource::FsCreds => (accessor_cred.fsuid(), accessor_cred.fsgid()),
            CredsSource::RealCreds => (accessor_cred.ruid(), accessor_cred.rgid()),
        };

        let target_cred = context.target().credentials();
        let caller_is_same = caller_uid == target_cred.euid()
            && caller_uid == target_cred.suid()
            && caller_uid == target_cred.ruid()
            && caller_gid == target_cred.egid()
            && caller_gid == target_cred.sgid()
            && caller_gid == target_cred.rgid();
        if caller_is_same || context.accessor_has_cap_sys_ptrace() {
            return Ok(());
        }

        return_errno_with_message!(
            Errno::EPERM,
            "the calling process does not have the required permissions"
        );
    }
}

impl LsmInodeHook for CapabilityLsm {
    fn on_inode_dac_override(&self, context: &InodeDacOverrideContext) -> Result<Permission> {
        let credentials = context.posix_thread().credentials();
        if !credentials
            .effective_capset()
            .contains(CapSet::DAC_OVERRIDE)
        {
            return Ok(Permission::empty());
        }

        let mut overridden = Permission::empty();
        let permission = context.permission();

        if permission.may_read() {
            overridden |= Permission::MAY_READ;
        }
        if permission.may_write() {
            overridden |= Permission::MAY_WRITE;
        }
        if permission.may_exec() {
            let mode = context.mode();
            if mode.is_owner_executable()
                || mode.is_group_executable()
                || mode.is_other_executable()
            {
                overridden |= Permission::MAY_EXEC;
            } else {
                return_errno_with_message!(
                    Errno::EACCES,
                    "root execute permission denied: no execute bits set"
                );
            }
        }

        Ok(overridden)
    }
}
