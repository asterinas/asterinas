// SPDX-License-Identifier: MPL-2.0

use super::profile::AppArmorProfileName;
use crate::prelude::*;

/// A set of AppArmor profiles attached to a task or object.
///
/// Linux AppArmor mediates labels rather than raw profile names. The current
/// implementation carries one profile per label, but the type is intentionally
/// shaped as a profile set so profile stacking can be added without changing
/// the surrounding task state and hook code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppArmorLabel {
    profiles: Vec<AppArmorProfileName>,
}

impl AppArmorLabel {
    /// Creates an unconfined label.
    pub fn new_unconfined() -> Self {
        Self {
            profiles: vec![AppArmorProfileName::new_unconfined()],
        }
    }

    /// Creates a label containing a single profile.
    pub fn new_single(profile_name: AppArmorProfileName) -> Self {
        Self {
            profiles: vec![profile_name],
        }
    }

    /// Returns the primary profile in this label.
    pub fn primary_profile(&self) -> &AppArmorProfileName {
        self.profiles
            .first()
            .expect("AppArmor labels always contain at least one profile")
    }

    /// Returns whether this is the unconfined label.
    pub fn is_unconfined(&self) -> bool {
        self.profiles.len() == 1 && self.primary_profile().is_unconfined()
    }
}

impl Default for AppArmorLabel {
    fn default() -> Self {
        Self::new_unconfined()
    }
}
