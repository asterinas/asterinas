// SPDX-License-Identifier: MPL-2.0

use bitflags::bitflags;

use crate::{
    prelude::*,
    process::{credentials::capabilities::CapSet, posix_thread::PosixThread},
};

/// Checks whether the current `PosixThread` may access the given target `PosixThread`
// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/kernel/ptrace.c#L276>
pub fn check_may_access(
    current_pthread: &PosixThread,
    target_pthread: &PosixThread,
    mode: PtraceMode,
) -> Result<()> {
    if mode.contains(PtraceMode::FSCREDS) == mode.contains(PtraceMode::REALCREDS) {
        return_errno_with_message!(
            Errno::EPERM,
            "should specify exactly one of FSCREDS and REALCREDS"
        );
    }

    if Weak::ptr_eq(
        current_pthread.weak_process(),
        target_pthread.weak_process(),
    ) {
        return Ok(());
    }

    let cred = current_pthread.credentials();
    let (caller_uid, caller_gid) = if mode.contains(PtraceMode::FSCREDS) {
        (cred.fsuid(), cred.fsgid())
    } else {
        (cred.ruid(), cred.rgid())
    };

    let tcred = target_pthread.credentials();
    let caller_is_same = caller_uid == tcred.euid()
        && caller_uid == tcred.suid()
        && caller_uid == tcred.ruid()
        && caller_gid == tcred.egid()
        && caller_gid == tcred.sgid()
        && caller_gid == tcred.rgid();
    let caller_has_cap = target_pthread
        .process()
        .user_ns()
        .lock()
        .check_cap(CapSet::SYS_PTRACE, current_pthread)
        .is_ok();

    if !caller_is_same && !caller_has_cap {
        return_errno_with_message!(
            Errno::EPERM,
            "the calling process does not have the required permissions"
        );
    }

    // TODO: Add further security checks (e.g., YAMA LSM).

    Ok(())
}

bitflags! {
    pub struct PtraceMode: u32 {
        const READ       = 0x01;
        const ATTACH     = 0x02;
        const NOAUDIT    = 0x04;
        const FSCREDS    = 0x08;
        const REALCREDS  = 0x10;
        const READ_FSCREDS     = Self::READ.bits()   | Self::FSCREDS.bits();
        const READ_REALCREDS   = Self::READ.bits()   | Self::REALCREDS.bits();
        const ATTACH_FSCREDS   = Self::ATTACH.bits() | Self::FSCREDS.bits();
        const ATTACH_REALCREDS = Self::ATTACH.bits() | Self::REALCREDS.bits();
    }
}
