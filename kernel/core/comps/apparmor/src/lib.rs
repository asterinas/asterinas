// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![deny(unsafe_code)]

/// Label enforcement mode for a profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileMode {
    Enforce,
    Complain,
}

/// AppArmor-style decision that may allow, deny, or report only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Decision {
    Allow,
    Deny,
    Complain,
}

/// Immutable Linux-capability allow list as a bitset for ids 0..=63.
#[derive(Clone, Copy, Debug)]
pub struct CapabilityRules {
    allowed: u64,
}

impl CapabilityRules {
    pub const fn allows(self, capability: u8) -> bool {
        if capability >= 64 {
            false
        } else {
            (self.allowed >> capability) & 1 == 1
        }
    }
}

/// Immutable AppArmor profile metadata used by M1.
#[derive(Clone, Copy, Debug)]
pub struct Profile {
    name: &'static str,
    mode: ProfileMode,
    capabilities: CapabilityRules,
}

impl Profile {
    pub const fn name(&self) -> &'static str {
        self.name
    }

    pub const fn mode(&self) -> ProfileMode {
        self.mode
    }

    pub const fn capabilities(&self) -> CapabilityRules {
        self.capabilities
    }
}

/// Copyable task label with exactly one profile reference.
#[derive(Clone, Copy)]
pub struct TaskLabel(&'static Profile);

impl TaskLabel {
    pub const fn new(profile: &'static Profile) -> Self {
        Self(profile)
    }

    pub const fn profile(&self) -> &'static Profile {
        self.0
    }

    pub const fn kernel_default() -> Self {
        Self::new(&KERNEL_DEFAULT_PROFILE)
    }
}

/// Kernel default profile that preserves historical behavior.
pub static KERNEL_DEFAULT_PROFILE: Profile = Profile {
    name: "kernel/default",
    mode: ProfileMode::Enforce,
    capabilities: CapabilityRules { allowed: u64::MAX },
};

/// Profile used by M1 capability regression checks.
pub static M1_TEST_PROFILE: Profile = Profile {
    name: "apparmor/m1-capability-test",
    mode: ProfileMode::Enforce,
    capabilities: CapabilityRules {
        allowed: u64::MAX & !(1u64 << 18),
    },
};

/// Complain-mode profile used by the M1 manual Guest walkthrough.
pub static M1_COMPLAIN_PROFILE: Profile = Profile {
    name: "apparmor/m1-complain-test",
    mode: ProfileMode::Complain,
    capabilities: CapabilityRules { allowed: 0 },
};

const APPARMOR_TEST_EXECUTABLE_PATH: &[u8] = b"/test/security/lsm/apparmor";
const MANUAL_ENFORCE_DENY_PATH: &[u8] = b"/test/security/lsm/apparmor-enforce-deny";
const MANUAL_ENFORCE_ALLOW_PATH: &[u8] = b"/test/security/lsm/apparmor-enforce-allow";
const MANUAL_FORK_PATH: &[u8] = b"/test/security/lsm/apparmor-fork";
const MANUAL_COMPLAIN_PATH: &[u8] = b"/test/security/lsm/apparmor-complain";

/// Decide whether a capability operation is allowed under the profile.
pub fn decide_capability(label: TaskLabel, capability: u8) -> Decision {
    if label.profile().capabilities().allows(capability) {
        Decision::Allow
    } else {
        match label.profile().mode() {
            ProfileMode::Enforce => Decision::Deny,
            ProfileMode::Complain => Decision::Complain,
        }
    }
}

/// Decide whether ptrace should be allowed by AppArmor profile relationship.
pub fn decide_ptrace(tracer: TaskLabel, tracee: TaskLabel) -> Decision {
    if core::ptr::eq(tracer.profile(), tracee.profile()) {
        Decision::Allow
    } else {
        match tracer.profile().mode() {
            ProfileMode::Enforce => Decision::Deny,
            ProfileMode::Complain => Decision::Complain,
        }
    }
}

/// Return a label transition target based on resolved executable path.
pub fn label_for_exec(current: TaskLabel, path: &[u8]) -> TaskLabel {
    if path == APPARMOR_TEST_EXECUTABLE_PATH
        || path == MANUAL_ENFORCE_DENY_PATH
        || path == MANUAL_ENFORCE_ALLOW_PATH
        || path == MANUAL_FORK_PATH
    {
        TaskLabel::new(&M1_TEST_PROFILE)
    } else if path == MANUAL_COMPLAIN_PATH {
        TaskLabel::new(&M1_COMPLAIN_PROFILE)
    } else {
        current
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn enforce_profile_denies_only_legacy_capability() {
        assert_eq!(
            super::decide_capability(super::TaskLabel::new(&super::M1_TEST_PROFILE), 18),
            super::Decision::Deny
        );
        assert_eq!(
            super::decide_capability(super::TaskLabel::new(&super::M1_TEST_PROFILE), 0),
            super::Decision::Allow
        );
        assert_eq!(
            super::decide_capability(super::TaskLabel::new(&super::M1_TEST_PROFILE), 64),
            super::Decision::Deny
        );
        assert_eq!(
            super::label_for_exec(
                super::TaskLabel::kernel_default(),
                b"/test/security/lsm/apparmor"
            )
            .profile()
            .name(),
            "apparmor/m1-capability-test"
        );
        assert_eq!(
            super::label_for_exec(super::TaskLabel::new(&super::M1_TEST_PROFILE), b"/bin/sh")
                .profile()
                .name(),
            "apparmor/m1-capability-test"
        );
    }

    #[test]
    fn ptrace_profile_mismatch_is_denied() {
        assert_eq!(
            super::decide_ptrace(
                super::TaskLabel::new(&super::M1_TEST_PROFILE),
                super::TaskLabel::new(&super::M1_TEST_PROFILE)
            ),
            super::Decision::Allow
        );
        assert_eq!(
            super::decide_ptrace(
                super::TaskLabel::kernel_default(),
                super::TaskLabel::kernel_default()
            ),
            super::Decision::Allow
        );
        assert_eq!(
            super::decide_ptrace(
                super::TaskLabel::kernel_default(),
                super::TaskLabel::new(&super::M1_TEST_PROFILE)
            ),
            super::Decision::Deny
        );
    }

    #[test]
    fn complain_profile_returns_complain_decision() {
        assert_eq!(
            super::decide_capability(
                super::TaskLabel::new(&super::M1_COMPLAIN_PROFILE),
                18
            ),
            super::Decision::Complain
        );
        assert_eq!(
            super::decide_ptrace(
                super::TaskLabel::new(&super::M1_COMPLAIN_PROFILE),
                super::TaskLabel::new(&super::M1_TEST_PROFILE)
            ),
            super::Decision::Complain
        );
    }
}
