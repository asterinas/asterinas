// SPDX-License-Identifier: MPL-2.0

//! Hooks for alien access checks.
//!
//! Linux names the corresponding LSM hook after `ptrace`, but Asterinas uses
//! `alien access` for permission checks against threads outside the caller's
//! thread group.

use super::super::modules;
use crate::{
    prelude::*,
    process::posix_thread::{PosixThread, alien_access::AlienAccessMode},
};

/// Defines hooks for alien access checks.
pub trait LsmAlienAccessCheck: Sync {
    /// Checks whether the accessor may inspect or attach to the target.
    fn alien_access_check(&self, _context: &AlienAccessContext) -> Result<()> {
        Ok(())
    }
}

/// The inputs for an alien access check through the LSM stack.
pub struct AlienAccessContext<'a> {
    accessor: &'a PosixThread,
    target: &'a PosixThread,
    mode: AlienAccessMode,
    accessor_has_cap_sys_ptrace: bool,
}

impl<'a> AlienAccessContext<'a> {
    pub const fn new(
        accessor: &'a PosixThread,
        target: &'a PosixThread,
        mode: AlienAccessMode,
        accessor_has_cap_sys_ptrace: bool,
    ) -> Self {
        Self {
            accessor,
            target,
            mode,
            accessor_has_cap_sys_ptrace,
        }
    }

    pub const fn accessor(&self) -> &PosixThread {
        self.accessor
    }

    pub const fn target(&self) -> &PosixThread {
        self.target
    }

    pub const fn mode(&self) -> AlienAccessMode {
        self.mode
    }

    pub const fn accessor_has_cap_sys_ptrace(&self) -> bool {
        self.accessor_has_cap_sys_ptrace
    }
}

/// Runs alien access hooks in module order.
pub fn alien_access_check(context: &AlienAccessContext) -> Result<()> {
    for module in modules::active_modules() {
        module.alien_access_check(context)?;
    }

    Ok(())
}
