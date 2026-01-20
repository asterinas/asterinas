// SPDX-License-Identifier: MPL-2.0

//! Alien access permission check for POSIX threads.
//!
//! An alien thread is one outside the current thread's thread group (the process).

use bitflags::bitflags;

use crate::{
    prelude::*,
    process::{credentials::capabilities::CapSet, posix_thread::PosixThread},
};

impl PosixThread {
    /// Checks whether `accessor` may access resources of `self`.
    ///
    /// NOTE: In Linux, the corresponding check is named `ptrace_may_access`,
    /// but not every call to it is actually related to `ptrace`.
    // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/kernel/ptrace.c#L276>.
    pub fn check_alien_access_from(
        &self,
        accessor: &PosixThread,
        mode: AlienAccessMode,
    ) -> Result<()> {
        if Weak::ptr_eq(accessor.weak_process(), self.weak_process()) {
            return Ok(());
        }

        let cred = accessor.credentials();
        let (caller_uid, caller_gid) = if mode.1 == CredsSource::FsCreds {
            (cred.fsuid(), cred.fsgid())
        } else {
            (cred.ruid(), cred.rgid())
        };

        let self_cred = self.credentials();
        let caller_is_same = caller_uid == self_cred.euid()
            && caller_uid == self_cred.suid()
            && caller_uid == self_cred.ruid()
            && caller_gid == self_cred.egid()
            && caller_gid == self_cred.sgid()
            && caller_gid == self_cred.rgid();
        let caller_has_cap = self
            .process()
            .user_ns()
            .lock()
            .check_cap(CapSet::SYS_PTRACE, accessor)
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
}

/// The mode used by the alien access permission check.
#[expect(dead_code)]
pub struct AlienAccessMode(AlienAccessFlags, CredsSource);

impl AlienAccessMode {
    /// Read-only alien access check, using real credentials (`ruid`/`rgid`).
    #[expect(dead_code)]
    pub const READ_WITH_REAL_CREDS: Self = Self(AlienAccessFlags::READ, CredsSource::RealCreds);
    /// Attach-level alien access check, using real credentials (`ruid`/`rgid`).
    pub const ATTACH_WITH_REAL_CREDS: Self = Self(AlienAccessFlags::ATTACH, CredsSource::RealCreds);
    /// Read-only alien access check, using filesystem credentials (`fsuid`/`fsgid`).
    pub const READ_WITH_FS_CREDS: Self = Self(AlienAccessFlags::READ, CredsSource::FsCreds);
    /// Attach-level alien access check, using filesystem credentials (`fsuid`/`fsgid`).
    pub const ATTACH_WITH_FS_CREDS: Self = Self(AlienAccessFlags::ATTACH, CredsSource::FsCreds);
}

bitflags! {
    /// Access strength in the alien access permission check.
    struct AlienAccessFlags: u32 {
        const READ       = 0x01;
        const ATTACH     = 0x02;
    }
}

/// The credentials used in the alien access permission check.
#[derive(PartialEq)]
enum CredsSource {
    FsCreds,
    RealCreds,
}
