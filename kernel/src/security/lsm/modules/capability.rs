// SPDX-License-Identifier: MPL-2.0

use super::super::{
    LsmFlags, LsmModule,
    hooks::{AlienAccessContext, CapableContext, LsmAlienAccessHook, LsmCapabilityHook},
};
use crate::{
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
        // Since creating new user namespaces is not supported at the moment,
        // there is effectively only one user namespace in the entire system.
        // Therefore, the thread has a single set of capabilities used for permission checks.
        // FIXME: Once support for creating new user namespaces is added,
        // we should verify the thread's capabilities within the relevant user namespace.
        if context
            .posix_thread()
            .credentials()
            .effective_capset()
            .contains(context.required_cap())
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
        if caller_is_same || {
            let target_process = context.target().process();
            let target_user_ns = target_process.user_ns().lock();
            self.on_capable(&CapableContext::new(
                target_user_ns.as_ref(),
                context.accessor(),
                CapSet::SYS_PTRACE,
            ))
            .is_ok()
        } {
            return Ok(());
        }

        return_errno_with_message!(
            Errno::EPERM,
            "the calling process does not have the required permissions"
        );
    }
}
