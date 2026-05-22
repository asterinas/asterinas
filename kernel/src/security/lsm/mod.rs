// SPDX-License-Identifier: MPL-2.0

//! The Linux Security Module (LSM) framework.
//!
//! LSM lets the kernel route security-sensitive operations through a stack of
//! built-in policy modules. Each module can implement shared hook traits and
//! inspect common hook contexts before allowing or rejecting an operation.
//!
//! This module defines the common LSM traits and hook contexts shared by
//! built-in modules such as `capability` and `yama`. Module selection follows
//! the `lsm=` and legacy `security=` kernel command-line parameters.

pub mod hooks;
mod modules;

pub mod yama {
    pub use super::modules::yama::{YamaScope, get_scope, set_scope};
}

pub use self::hooks::{
    AlienAccessContext, CapabilityReason, CapableContext, InodeDacOverrideContext,
};
use self::hooks::{LsmAlienAccessHook, LsmCapabilityHook, LsmInodeHook};
use crate::{fs::file::Permission, prelude::*};

bitflags! {
    /// LSM module flags.
    pub struct LsmFlags: u32 {
        /// Marks a module as selectable through the legacy `security=` parameter.
        const LEGACY_MAJOR = 1 << 0;
        /// Marks a module as mutually exclusive with other exclusive modules.
        const EXCLUSIVE = 1 << 1;
    }
}

/// The common interface for built-in LSM modules.
trait LsmModule: LsmAlienAccessHook + LsmCapabilityHook + LsmInodeHook + Sync {
    /// Returns the module name.
    fn name(&self) -> &'static str;

    /// Returns the module flags.
    fn flags(&self) -> LsmFlags;
}

/// Returns whether the Yama LSM is enabled.
pub fn is_yama_enabled() -> bool {
    modules::active_modules()
        .iter()
        .any(|module| module.name() == "yama")
}

pub(super) fn init() {
    for module in modules::active_modules() {
        info!("[kernel] LSM module enabled: {}", module.name());
    }
}

/// Runs the LSM stack for a capability check.
pub fn capable(context: &CapableContext) -> Result<()> {
    hooks::on_capable(context)
}

/// Runs the LSM stack for a DAC override decision on an inode.
pub fn inode_dac_override(context: &InodeDacOverrideContext) -> Result<Permission> {
    hooks::on_inode_dac_override(context)
}
