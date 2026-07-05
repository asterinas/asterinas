// SPDX-License-Identifier: MPL-2.0

use crate::process::credentials::capabilities::CapSet;

/// AppArmor capability policy attached to a profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct AppArmorCapabilityPolicy {
    allowed: CapSet,
    audit: CapSet,
    quiet: CapSet,
}

impl AppArmorCapabilityPolicy {
    /// Creates capability policy from Linux AppArmor capability masks.
    pub const fn new(allowed: CapSet, audit: CapSet, quiet: CapSet) -> Self {
        Self {
            allowed,
            audit,
            quiet,
        }
    }

    /// Returns whether the requested capabilities are allowed.
    pub fn allows(self, capabilities: CapSet) -> bool {
        self.allowed.contains(capabilities)
    }

    /// Returns capability audit mask.
    pub const fn audit(self) -> CapSet {
        self.audit
    }

    /// Returns capability quiet mask.
    pub const fn quiet(self) -> CapSet {
        self.quiet
    }
}

impl Default for AppArmorCapabilityPolicy {
    fn default() -> Self {
        Self::new(CapSet::empty(), CapSet::empty(), CapSet::empty())
    }
}
