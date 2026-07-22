// SPDX-License-Identifier: MPL-2.0

//! AppArmor profile-identity and label foundations.
//!
//! This module registers AppArmor as an optional major LSM.
//! It defines profile-identity, label, and task-state types for later policy enforcement.
//! The initial module exposes no security hooks.
//! Selecting it does not change authorization results.

mod label;
mod profile;
mod state;

pub use self::state::AppArmorTaskState;
use super::super::{
    LsmFlags, LsmModule,
    hooks::{LsmAlienAccessHook, LsmCapabilityHook},
};

pub(super) static APPARMOR_LSM: AppArmorLsm = AppArmorLsm;

/// The AppArmor major LSM.
pub(super) struct AppArmorLsm;

impl LsmModule for AppArmorLsm {
    fn name(&self) -> &'static str {
        "apparmor"
    }

    fn flags(&self) -> LsmFlags {
        LsmFlags::LEGACY_MAJOR | LsmFlags::EXCLUSIVE
    }

    fn alien_access_hook(&self) -> Option<&dyn LsmAlienAccessHook> {
        None
    }

    fn capability_hook(&self) -> Option<&dyn LsmCapabilityHook> {
        None
    }
}
