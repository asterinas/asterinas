// SPDX-License-Identifier: MPL-2.0

//! The Linux Security Module (LSM) framework.
//!
//! LSM lets the kernel route security-sensitive operations through a stack of
//! built-in policy modules. Each module can implement shared hook traits and
//! inspect common hook contexts before allowing or rejecting an operation.
//!
//! This module defines the common LSM traits, alien access hook contexts, and
//! dispatch helpers shared by built-in modules such as `yama`.

mod checks;
mod modules;

pub use self::{
    checks::alien_access::{AlienAccessContext, LsmAlienAccessCheck, alien_access_check},
    modules::yama::{YamaScope, get_yama_scope, set_yama_scope},
};
use crate::prelude::*;

/// The kind of an LSM module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LsmKind {
    Minor,
    #[expect(dead_code)]
    Major,
}

impl LsmKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Minor => "minor",
            Self::Major => "major",
        }
    }
}

/// Defines the common interface for built-in LSM modules.
pub trait LsmModule: LsmAlienAccessCheck + Sync {
    /// Returns the module name.
    fn name(&self) -> &'static str;

    /// Returns the module kind.
    fn kind(&self) -> LsmKind;

    /// Initializes the module during boot.
    fn init(&self);
}

pub(super) fn init() {
    for module in modules::active_modules() {
        info!(
            "[kernel] LSM module enabled: {} ({})",
            module.name(),
            module.kind().as_str()
        );
        module.init();
    }
}
