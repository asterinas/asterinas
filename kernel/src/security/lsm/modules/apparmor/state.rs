// SPDX-License-Identifier: MPL-2.0

use super::{label::AppArmorLabel, profile::AppArmorProfileName};
use crate::prelude::*;

/// AppArmor state attached to a task.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppArmorTaskState {
    label: AppArmorLabel,
    onexec_profile: Option<AppArmorProfileName>,
    previous_profile: Option<AppArmorProfileName>,
    mode: AppArmorMode,
}

impl AppArmorTaskState {
    /// Creates the default unconfined AppArmor task state.
    pub fn new_unconfined() -> Self {
        Self {
            label: AppArmorLabel::new_unconfined(),
            onexec_profile: None,
            previous_profile: None,
            mode: AppArmorMode::Enforce,
        }
    }

    /// Creates a copy with a profile requested for the next `execve`.
    pub fn with_onexec_profile(mut self, profile_name: Option<AppArmorProfileName>) -> Self {
        self.onexec_profile = profile_name;
        self
    }

    /// Creates task state after an executable profile transition.
    pub fn transition_to(&self, profile_name: AppArmorProfileName, mode: AppArmorMode) -> Self {
        Self {
            label: AppArmorLabel::new_single(profile_name),
            onexec_profile: None,
            previous_profile: Some(self.current_profile().clone()),
            mode,
        }
    }

    /// Creates task state after an immediate profile change.
    pub fn change_to(&self, profile_name: AppArmorProfileName, mode: AppArmorMode) -> Self {
        self.transition_to(profile_name, mode)
    }

    /// Returns the current profile.
    pub fn current_profile(&self) -> &AppArmorProfileName {
        self.label.primary_profile()
    }

    /// Returns whether the current task label is unconfined.
    pub fn is_unconfined(&self) -> bool {
        self.label.is_unconfined()
    }

    /// Returns the profile requested for the next `execve`.
    pub fn onexec_profile(&self) -> Option<&AppArmorProfileName> {
        self.onexec_profile.as_ref()
    }

    /// Returns the previous profile.
    pub fn previous_profile(&self) -> Option<&AppArmorProfileName> {
        self.previous_profile.as_ref()
    }

    /// Returns the current profile mode.
    pub fn mode(&self) -> AppArmorMode {
        self.mode
    }
}

impl Default for AppArmorTaskState {
    fn default() -> Self {
        Self::new_unconfined()
    }
}

/// The enforcement mode of an AppArmor profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppArmorMode {
    /// Denied operations fail.
    Enforce,
    /// Denied operations are logged but allowed.
    Complain,
}

impl AppArmorMode {
    /// Parses an AppArmor enforcement mode.
    pub fn parse(mode: &str) -> Result<Self> {
        match mode {
            "enforce" => Ok(Self::Enforce),
            "complain" => Ok(Self::Complain),
            _ => return_errno_with_message!(Errno::EINVAL, "the AppArmor mode is invalid"),
        }
    }

    /// Returns the mode text.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Enforce => "enforce",
            Self::Complain => "complain",
        }
    }
}
