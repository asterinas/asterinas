// SPDX-License-Identifier: MPL-2.0

use super::super::modules;
use crate::{
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::PosixThread},
};

/// Runs capability hooks in module order.
pub fn on_capable(context: CapableContext) -> Result<()> {
    for module in modules::active_modules() {
        module.on_capable(&context)?;
    }

    Ok(())
}

/// The inputs for checking whether a thread has a capability.
pub struct CapableContext<'a> {
    target_user_ns: &'a UserNamespace,
    posix_thread: &'a PosixThread,
    required_cap: CapSet,
}

impl<'a> CapableContext<'a> {
    /// Creates a capability check context.
    pub const fn new(
        target_user_ns: &'a UserNamespace,
        posix_thread: &'a PosixThread,
        required_cap: CapSet,
    ) -> Self {
        Self {
            target_user_ns,
            posix_thread,
            required_cap,
        }
    }

    /// Returns the user namespace against which the capability is checked.
    #[expect(
        dead_code,
        reason = "Asterinas currently has only the initial user namespace"
    )]
    pub const fn target_user_ns(&self) -> &UserNamespace {
        self.target_user_ns
    }

    /// Returns the thread whose credentials are checked.
    pub const fn posix_thread(&self) -> &PosixThread {
        self.posix_thread
    }

    /// Returns the required capability.
    pub const fn required_cap(&self) -> CapSet {
        self.required_cap
    }
}
