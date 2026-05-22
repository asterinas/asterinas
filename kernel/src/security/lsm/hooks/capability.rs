// SPDX-License-Identifier: MPL-2.0

use super::super::modules;
use crate::{
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::PosixThread},
};

/// Runs capability hooks in module order.
pub fn on_capable(context: &CapableContext) -> Result<()> {
    for module in modules::active_modules() {
        module.on_capable(context)?;
    }

    Ok(())
}

/// The inputs for checking whether a thread has a capability.
pub struct CapableContext<'a> {
    user_namespace: &'a UserNamespace,
    posix_thread: &'a PosixThread,
    capability: CapSet,
    reason: CapabilityReason,
}

impl<'a> CapableContext<'a> {
    /// Creates a capability check context.
    pub const fn new(
        user_namespace: &'a UserNamespace,
        posix_thread: &'a PosixThread,
        capability: CapSet,
        reason: CapabilityReason,
    ) -> Self {
        Self {
            user_namespace,
            posix_thread,
            capability,
            reason,
        }
    }

    /// Returns the user namespace where the capability is checked.
    pub const fn user_namespace(&self) -> &UserNamespace {
        self.user_namespace
    }

    /// Returns the thread whose credentials are checked.
    pub const fn posix_thread(&self) -> &PosixThread {
        self.posix_thread
    }

    /// Returns the required capability.
    pub const fn capability(&self) -> CapSet {
        self.capability
    }

    /// Returns the kernel operation that requires the capability.
    pub const fn reason(&self) -> CapabilityReason {
        self.reason
    }
}

/// The kernel operation that requires a capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityReason {
    CredentialsSetUid,
    CredentialsSetGid,
    CredentialsSetPcap,
    Namespace,
    Ptrace,
    ResourceLimit,
    Reboot,
    Signal,
    Socket,
    Xattr,
}
