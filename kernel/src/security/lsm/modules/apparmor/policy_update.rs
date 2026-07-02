// SPDX-License-Identifier: MPL-2.0

use super::profile::{AppArmorProfile, AppArmorProfileName};
use crate::prelude::*;

/// A policy update decoded from a user-space policy payload.
pub(super) enum AppArmorPolicyUpdate {
    /// Inserts or replaces a profile.
    Replace(Box<AppArmorProfile>),
    /// Inserts or replaces multiple profiles decoded from one binary payload.
    ReplaceMany(Vec<AppArmorProfile>),
    /// Removes a profile.
    Remove(AppArmorProfileName),
}
