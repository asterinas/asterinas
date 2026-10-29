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

mod cred;
pub mod hooks;
mod modules;

pub(crate) use cred::CredSecurity;
pub use cred::LsmState;

pub(crate) mod yama {
    pub(crate) use super::modules::yama::{YamaScope, get_scope, set_scope};
}

use self::hooks::{LsmAlienAccessHook, LsmCapabilityHook};
use crate::prelude::*;

bitflags! {
    /// LSM module flags.
    pub struct LsmFlags: u32 {
        /// Marks a module as selectable through the legacy `security=` parameter.
        const LEGACY_MAJOR = 1 << 0;
        /// Marks a module as mutually exclusive with other exclusive modules.
        const EXCLUSIVE = 1 << 1;
    }
}

/// The common interface for LSM modules.
pub trait LsmModule: LsmAlienAccessHook + LsmCapabilityHook + Sync {
    /// Returns the module name.
    fn name(&self) -> &'static str;

    /// Returns the module flags.
    fn flags(&self) -> LsmFlags;

    /// Creates the module's initial credential state.
    /// `None` needs no allocation.
    fn init_cred_state(&self) -> Result<Option<LsmState>> {
        Ok(None)
    }

    /// Formats the current task attribute, or returns `None` if unsupported.
    fn current_attr(&self, _state: Option<&LsmState>) -> Result<Option<String>> {
        Ok(None)
    }

    /// Returns whether exec preparation needs the original executable's resolved pathname.
    fn needs_exec_path(&self, _current: Option<&LsmState>) -> bool {
        false
    }

    /// Prepares immutable state without modifying the task's committed credentials.
    /// An unavailable path is distinct from an error preparing a state.
    fn prepare_exec_state(
        &self,
        current: Option<&LsmState>,
        _path: Option<&str>,
    ) -> Result<Option<LsmState>> {
        Ok(current.cloned())
    }

    /// Initializes the module after it has been selected.
    fn init(&self) -> Result<()> {
        Ok(())
    }
}

/// Registers an LSM module before the LSM stack is initialized.
pub fn register(module: &'static dyn LsmModule) -> Result<()> {
    modules::register(module)
}

/// Returns whether the Yama LSM is enabled.
pub(crate) fn is_yama_enabled() -> bool {
    modules::active_modules()
        .iter()
        .any(|module| module.name() == "yama")
}

pub(super) fn init() -> Result<()> {
    for module in modules::active_modules() {
        info!("[kernel] LSM module enabled: {}", module.name());
        module.init()?;
    }

    Ok(())
}
