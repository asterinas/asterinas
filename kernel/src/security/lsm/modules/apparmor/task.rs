// SPDX-License-Identifier: MPL-2.0

use super::profile::AppArmorProfileName;
use crate::prelude::*;

bitflags! {
    /// Task-to-task permissions mediated by AppArmor.
    #[derive(Default)]
    pub(super) struct AppArmorTaskPermission: u32 {
        /// Allows read-like cross-task inspection.
        const PTRACE_READ = 1 << 0;
        /// Allows attach-like cross-task control.
        const PTRACE_TRACE = 1 << 1;
        /// Allows sending signals or checking signal permission.
        const SIGNAL = 1 << 2;
    }
}

/// A peer selector for task-to-task AppArmor rules.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum AppArmorTaskPeer {
    /// Matches any peer profile.
    Any,
    /// Matches one peer profile.
    Profile(AppArmorProfileName),
}

/// A task-to-task AppArmor allow rule.
#[derive(Clone, Debug)]
pub(super) struct AppArmorTaskRule {
    peer: AppArmorTaskPeer,
    permissions: AppArmorTaskPermission,
}

impl AppArmorTaskRule {
    /// Creates a task rule.
    pub(super) fn new(peer: AppArmorTaskPeer, permissions: AppArmorTaskPermission) -> Self {
        Self { peer, permissions }
    }

    fn matches(&self, peer_profile: &AppArmorProfileName) -> bool {
        match &self.peer {
            AppArmorTaskPeer::Any => true,
            AppArmorTaskPeer::Profile(profile_name) => profile_name == peer_profile,
        }
    }
}

/// Task-to-task policy attached to an AppArmor profile.
#[derive(Clone, Debug, Default)]
pub(super) struct AppArmorTaskPolicy {
    rules: Vec<AppArmorTaskRule>,
}

impl AppArmorTaskPolicy {
    /// Creates a task policy from allow rules.
    pub(super) fn new(rules: Vec<AppArmorTaskRule>) -> Self {
        Self { rules }
    }

    /// Evaluates task-to-task access for a peer profile.
    pub(super) fn evaluate_access(
        &self,
        peer_profile: &AppArmorProfileName,
        permissions: AppArmorTaskPermission,
    ) -> AppArmorTaskAccessOutcome {
        let mut allowed = AppArmorTaskPermission::empty();
        for rule in &self.rules {
            if rule.matches(peer_profile) {
                allowed |= rule.permissions;
            }
        }

        AppArmorTaskAccessOutcome {
            denied: permissions - allowed,
        }
    }
}

/// A task-to-task access decision from a profile.
pub(super) struct AppArmorTaskAccessOutcome {
    /// Permissions denied by the policy.
    pub(super) denied: AppArmorTaskPermission,
}
