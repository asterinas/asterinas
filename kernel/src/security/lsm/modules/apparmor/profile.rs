// SPDX-License-Identifier: MPL-2.0

use super::{
    attachment::AppArmorAttachment,
    capability::AppArmorCapabilityPolicy,
    dfa::{AppArmorDfaAccessOutcome, AppArmorDfaFilePolicy},
    path::{AppArmorExecTransition, AppArmorFilePermission, AppArmorPathRule, AppArmorPathView},
    state::AppArmorMode,
};
use crate::{prelude::*, process::credentials::capabilities::CapSet};

/// The name of an AppArmor profile.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AppArmorProfileName(String);

impl AppArmorProfileName {
    /// The default profile before policy-driven transitions exist.
    pub const UNCONFINED: &'static str = "unconfined";

    /// Creates a profile name.
    pub fn new(name: String) -> Result<Self> {
        if name.is_empty() {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor profile name is empty");
        }

        Ok(Self(name))
    }

    /// Creates the default unconfined profile name.
    pub fn new_unconfined() -> Self {
        Self(String::from(Self::UNCONFINED))
    }

    /// Returns whether this is the default unconfined profile.
    pub fn is_unconfined(&self) -> bool {
        self.as_str() == Self::UNCONFINED
    }

    /// Returns the profile name text.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl Default for AppArmorProfileName {
    fn default() -> Self {
        Self::new_unconfined()
    }
}

/// An AppArmor profile.
#[derive(Clone, Debug)]
pub struct AppArmorProfile {
    name: AppArmorProfileName,
    attachment: AppArmorAttachment,
    mode: AppArmorMode,
    file_policy: AppArmorFilePolicy,
    capability_policy: AppArmorCapabilityPolicy,
    transition_policy: AppArmorProfileTransitionPolicy,
}

impl AppArmorProfile {
    /// Creates a profile.
    pub fn new(
        name: AppArmorProfileName,
        mode: AppArmorMode,
        file_rules: Vec<AppArmorPathRule>,
    ) -> Self {
        Self::new_with_file_policy(name, mode, AppArmorFilePolicy::PathRules(file_rules))
    }

    /// Creates a profile with an explicit file policy backend.
    pub(super) fn new_with_file_policy(
        name: AppArmorProfileName,
        mode: AppArmorMode,
        file_policy: AppArmorFilePolicy,
    ) -> Self {
        let attachment = AppArmorAttachment::from_profile(&name, None, None);
        Self::new_with_policies(
            name,
            attachment,
            mode,
            file_policy,
            AppArmorCapabilityPolicy::default(),
        )
    }

    /// Creates a profile with explicit attachment and file policy backends.
    pub(super) fn new_with_policies(
        name: AppArmorProfileName,
        attachment: AppArmorAttachment,
        mode: AppArmorMode,
        file_policy: AppArmorFilePolicy,
        capability_policy: AppArmorCapabilityPolicy,
    ) -> Self {
        Self::new_with_transition_policy(
            name,
            attachment,
            mode,
            file_policy,
            capability_policy,
            AppArmorProfileTransitionPolicy::default(),
        )
    }

    /// Creates a profile with explicit policy backends and transition rules.
    pub(super) fn new_with_transition_policy(
        name: AppArmorProfileName,
        attachment: AppArmorAttachment,
        mode: AppArmorMode,
        file_policy: AppArmorFilePolicy,
        capability_policy: AppArmorCapabilityPolicy,
        transition_policy: AppArmorProfileTransitionPolicy,
    ) -> Self {
        Self {
            name,
            attachment,
            mode,
            file_policy,
            capability_policy,
            transition_policy,
        }
    }

    /// Creates the default unconfined profile.
    pub fn new_unconfined() -> Self {
        Self {
            name: AppArmorProfileName::new_unconfined(),
            attachment: AppArmorAttachment::new(None, None),
            mode: AppArmorMode::Enforce,
            file_policy: AppArmorFilePolicy::PathRules(Vec::new()),
            capability_policy: AppArmorCapabilityPolicy::default(),
            transition_policy: AppArmorProfileTransitionPolicy::default(),
        }
    }

    /// Returns the profile name.
    pub fn name(&self) -> &AppArmorProfileName {
        &self.name
    }

    /// Returns the profile mode.
    pub fn mode(&self) -> AppArmorMode {
        self.mode
    }

    /// Returns whether this profile is attached to a path.
    pub(super) fn matches_attachment(&self, path_view: &AppArmorPathView) -> bool {
        self.attachment.matches(path_view)
    }

    /// Evaluates file access for this profile.
    pub fn evaluate_file_access(
        &self,
        path_view: &AppArmorPathView,
        permissions: AppArmorFilePermission,
    ) -> Result<AppArmorFileAccessOutcome> {
        match &self.file_policy {
            AppArmorFilePolicy::PathRules(rules) => {
                Ok(evaluate_path_rules(rules, path_view, permissions))
            }
            AppArmorFilePolicy::Dfa(policy) => policy
                .evaluate_path_access(path_view, permissions)
                .map(AppArmorFileAccessOutcome::from),
        }
    }

    /// Evaluates capability access for this profile.
    pub fn evaluate_capability_access(&self, capabilities: CapSet) -> AppArmorCapabilityOutcome {
        AppArmorCapabilityOutcome {
            denied: if self.capability_policy.allows(capabilities) {
                CapSet::empty()
            } else {
                capabilities
            },
        }
    }

    /// Returns whether this profile allows changing to `target`.
    pub fn allows_profile_transition(
        &self,
        target: &AppArmorProfileName,
        kind: AppArmorProfileTransitionKind,
    ) -> bool {
        self.transition_policy.allows(target, kind)
    }
}

/// The file-policy backend used by an AppArmor profile.
#[derive(Clone, Debug)]
pub(super) enum AppArmorFilePolicy {
    /// The temporary Asterinas text/debug loader rule list.
    PathRules(Vec<AppArmorPathRule>),
    /// Linux AppArmor DFA policy decoded from binary policy.
    Dfa(Box<AppArmorDfaFilePolicy>),
}

/// A file-access decision from a profile.
pub struct AppArmorFileAccessOutcome {
    /// Permissions denied by the policy.
    pub denied: AppArmorFilePermission,
    /// Executable profile transition selected by the matching rule.
    pub exec_transition: AppArmorExecTransition,
    /// Whether matching permissions requested auditing.
    pub audit: bool,
}

/// A capability-access decision from a profile.
pub struct AppArmorCapabilityOutcome {
    /// Capabilities denied by the policy.
    pub denied: CapSet,
}

/// A task profile transition requested through procfs attributes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppArmorProfileTransitionKind {
    /// Immediately changes the current task profile.
    ChangeProfile,
    /// Sets the profile to apply at the next successful `execve`.
    ChangeOnexec,
}

/// Profile-to-profile transition rules attached to an AppArmor profile.
#[derive(Clone, Debug, Default)]
pub(super) struct AppArmorProfileTransitionPolicy {
    change_profile: Vec<AppArmorProfileName>,
    change_onexec: Vec<AppArmorProfileName>,
}

impl AppArmorProfileTransitionPolicy {
    /// Creates a profile transition policy.
    pub(super) fn new(
        change_profile: Vec<AppArmorProfileName>,
        change_onexec: Vec<AppArmorProfileName>,
    ) -> Self {
        Self {
            change_profile,
            change_onexec,
        }
    }

    fn allows(&self, target: &AppArmorProfileName, kind: AppArmorProfileTransitionKind) -> bool {
        let allowed_targets = match kind {
            AppArmorProfileTransitionKind::ChangeProfile => &self.change_profile,
            AppArmorProfileTransitionKind::ChangeOnexec => &self.change_onexec,
        };

        allowed_targets.iter().any(|allowed| allowed == target)
    }
}

impl From<AppArmorDfaAccessOutcome> for AppArmorFileAccessOutcome {
    fn from(outcome: AppArmorDfaAccessOutcome) -> Self {
        Self {
            denied: outcome.denied,
            exec_transition: outcome.exec_transition,
            audit: outcome.audit,
        }
    }
}

fn evaluate_path_rules(
    rules: &[AppArmorPathRule],
    path_view: &AppArmorPathView,
    permissions: AppArmorFilePermission,
) -> AppArmorFileAccessOutcome {
    let mut allowed = AppArmorFilePermission::empty();
    let mut denied = AppArmorFilePermission::empty();
    let mut exec_transition = AppArmorExecTransition::Inherit;
    let mut audit = false;

    for rule in rules {
        if !rule.matches(path_view) {
            continue;
        }

        let matched_permissions = rule.permissions() & permissions;
        if matched_permissions.is_empty() {
            continue;
        }

        audit |= rule.audit();
        if rule.deny() {
            denied |= matched_permissions;
            continue;
        }

        allowed |= matched_permissions;
        if matched_permissions.contains(AppArmorFilePermission::EXECUTE) {
            exec_transition = rule.exec_transition().clone();
        }
    }

    let missing = permissions - allowed;
    AppArmorFileAccessOutcome {
        denied: denied | missing,
        exec_transition,
        audit,
    }
}
