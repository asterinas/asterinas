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

        yama::check_alien_access(accessor, self, mode, caller_has_cap)?;

        Ok(())
    }
}

/// The mode used by the alien access permission check.
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

// TODO: Rewrite Yama as a minor LSM module when Asterinas introduces the LSM subsystem.
pub mod yama {
    use core::sync::atomic::{AtomicI32, Ordering};

    use super::*;
    use crate::process::{Process, UserNamespace, posix_thread::AsPosixThread};

    /// Returns the current Yama scope for alien access.
    pub fn get_yama_scope() -> YamaScope {
        YAMA_SCOPE.load(Ordering::Relaxed).try_into().unwrap()
    }

    /// Sets the Yama scope for alien access.
    pub fn set_yama_scope(new_scope: YamaScope) -> Result<()> {
        UserNamespace::get_init_singleton().check_cap(
            CapSet::SYS_PTRACE,
            current_thread!().as_posix_thread().unwrap(),
        )?;

        YAMA_SCOPE
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current_scope| {
                let is_downgrading_from_no_attach =
                    current_scope == YamaScope::NoAttach as i32 && new_scope != YamaScope::NoAttach;
                (!is_downgrading_from_no_attach).then_some(new_scope as i32)
            })
            .map_err(|_| {
                Error::with_message(
                    Errno::EINVAL,
                    "`YamaScope::NoAttach` cannot be changed once set",
                )
            })?;

        Ok(())
    }

    static YAMA_SCOPE: AtomicI32 = AtomicI32::new(YamaScope::Relational as i32);

    /// The Yama scope levels.
    #[repr(i32)]
    #[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
    pub enum YamaScope {
        /// No additional restrictions on alien attach.
        Disabled = 0,
        /// Only allow alien attach by ancestor processes, or processes with `CapSet::SYS_PTRACE`.
        Relational = 1,
        /// Only allow alien attach by processes with `CapSet::SYS_PTRACE`.
        Capability = 2,
        /// Disallow any alien attach.
        NoAttach = 3,
    }

    pub(super) fn check_alien_access(
        accessor: &PosixThread,
        target: &PosixThread,
        mode: AlienAccessMode,
        caller_has_cap: bool,
    ) -> Result<()> {
        if !mode.0.contains(AlienAccessFlags::ATTACH) {
            return Ok(());
        }

        let is_denied = match get_yama_scope() {
            YamaScope::Disabled => false,
            YamaScope::Relational => {
                !caller_has_cap && !is_ancestor_of(accessor.weak_process(), target.process())
            }
            YamaScope::Capability => !caller_has_cap,
            YamaScope::NoAttach => true,
        };

        if is_denied {
            return_errno_with_message!(Errno::EPERM, "alien access is denied due to Yama scope");
        }

        Ok(())
    }

    fn is_ancestor_of(ancestor: &Weak<Process>, descendant: Arc<Process>) -> bool {
        let mut current = descendant;
        loop {
            let parent_guard = current.parent().lock();
            let parent = parent_guard.process();
            if Weak::ptr_eq(parent, ancestor) {
                return true;
            }
            let Some(parent) = parent.upgrade() else {
                return false;
            };
            drop(parent_guard);
            current = parent;
        }
    }
}
