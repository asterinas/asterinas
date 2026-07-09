// SPDX-License-Identifier: MPL-2.0

//! Hooks for signal permission checks.

use super::super::modules;
use crate::{
    prelude::*,
    process::{posix_thread::PosixThread, signal::sig_num::SigNum},
};

/// Runs signal hooks in module order.
pub fn on_signal(context: &SignalContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.on_signal(context)?;
    }

    Ok(())
}

/// The inputs for a signal permission check through the LSM stack.
pub struct SignalContext<'a> {
    sender: &'a PosixThread,
    target: &'a PosixThread,
    signum: Option<SigNum>,
}

impl<'a> SignalContext<'a> {
    /// Creates a signal permission context.
    pub const fn new(
        sender: &'a PosixThread,
        target: &'a PosixThread,
        signum: Option<SigNum>,
    ) -> Self {
        Self {
            sender,
            target,
            signum,
        }
    }

    /// Returns the thread sending the signal.
    pub const fn sender(&self) -> &PosixThread {
        self.sender
    }

    /// Returns the thread receiving the signal.
    pub const fn target(&self) -> &PosixThread {
        self.target
    }

    /// Returns the signal number, or `None` for permission-only checks.
    pub const fn signum(&self) -> Option<SigNum> {
        self.signum
    }
}
