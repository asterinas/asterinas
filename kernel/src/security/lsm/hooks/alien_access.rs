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

/// Runs alien access hooks in module order.
pub fn on_alien_access(context: AlienAccessContext) -> Result<()> {
    for module in modules::active_modules() {
        module.on_alien_access(&context)?;
    }

    Ok(())
}

/// The inputs for an alien access check through the LSM stack.
pub struct AlienAccessContext<'a> {
    accessor: &'a PosixThread,
    target: &'a PosixThread,
    mode: AlienAccessMode,
}

impl<'a> AlienAccessContext<'a> {
    /// Creates an alien access context.
    pub const fn new(
        accessor: &'a PosixThread,
        target: &'a PosixThread,
        mode: AlienAccessMode,
    ) -> Self {
        Self {
            accessor,
            target,
            mode,
        }
    }

    /// Returns the thread requesting access.
    pub const fn accessor(&self) -> &PosixThread {
        self.accessor
    }

    /// Returns the thread being accessed.
    pub const fn target(&self) -> &PosixThread {
        self.target
    }

    /// Returns the requested access mode.
    pub const fn mode(&self) -> AlienAccessMode {
        self.mode
    }
}
