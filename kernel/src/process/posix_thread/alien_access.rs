// SPDX-License-Identifier: MPL-2.0

//! Alien access permission check for POSIX threads.
//!
//! An alien thread is one outside the current thread's thread group (the process).

use crate::{prelude::*, process::posix_thread::PosixThread, security::lsm::hooks as lsm_hooks};

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

        lsm_hooks::on_alien_access(lsm_hooks::AlienAccessContext::new(accessor, self, mode))
    }
}

/// The credentials used by an alien access check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredsSource {
    FsCreds,
    RealCreds,
}

/// The strength of an alien access check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlienAccessKind {
    Read,
    Attach,
}

/// An alien access check mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AlienAccessMode {
    kind: AlienAccessKind,
    creds: CredsSource,
}

impl AlienAccessMode {
    /// Read-only alien access check using real credentials.
    #[expect(dead_code)]
    pub const READ_WITH_REAL_CREDS: Self = Self::new(AlienAccessKind::Read, CredsSource::RealCreds);
    /// Attach-level alien access check using real credentials.
    pub const ATTACH_WITH_REAL_CREDS: Self =
        Self::new(AlienAccessKind::Attach, CredsSource::RealCreds);
    /// Read-only alien access check using filesystem credentials.
    pub const READ_WITH_FS_CREDS: Self = Self::new(AlienAccessKind::Read, CredsSource::FsCreds);
    /// Attach-level alien access check using filesystem credentials.
    pub const ATTACH_WITH_FS_CREDS: Self = Self::new(AlienAccessKind::Attach, CredsSource::FsCreds);

    pub const fn new(kind: AlienAccessKind, creds: CredsSource) -> Self {
        Self { kind, creds }
    }

    pub const fn kind(self) -> AlienAccessKind {
        self.kind
    }

    pub const fn creds(self) -> CredsSource {
        self.creds
    }
}
