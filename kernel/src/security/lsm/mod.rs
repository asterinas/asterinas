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

use aster_systree::SysObj;

use self::hooks::{LsmAlienAccessHook, LsmCapabilityHook, LsmFileOpenHook};
use crate::{prelude::*, process::posix_thread::PosixThread};

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
trait LsmModule: Sync {
    /// Returns the module name.
    fn name(&self) -> &'static str;

    /// Returns the module flags.
    fn flags(&self) -> LsmFlags;

    /// Returns the module's alien-access hook, if it implements one.
    fn alien_access_hook(&self) -> Option<&dyn LsmAlienAccessHook> {
        None
    }

    /// Returns the module's capability hook, if it implements one.
    fn capability_hook(&self) -> Option<&dyn LsmCapabilityHook> {
        None
    }

    /// Returns the module's file-open hook, if it implements one.
    fn file_open_hook(&self) -> Option<&dyn LsmFileOpenHook> {
        None
    }

    /// Returns the module's task-attribute interface, if it has one.
    fn task_attrs(&self) -> Option<&dyn LsmTaskAttrs> {
        None
    }

    /// Returns the module's top-level securityfs node, if it has one.
    fn securityfs_node(&self) -> Option<Arc<dyn SysObj>> {
        None
    }
}

/// An LSM interface exposed through `/proc/<pid>/attr`.
trait LsmTaskAttrs: Sync {
    /// Returns the module's `current` task attribute.
    fn current(&self, posix_thread: &PosixThread) -> Result<String>;

    /// Updates the module's `current` task attribute.
    fn set_current(&self, posix_thread: &PosixThread, value: &str) -> Result<()>;
}

/// Returns whether the Yama LSM is enabled.
pub fn is_yama_enabled() -> bool {
    modules::active_modules()
        .iter()
        .any(|module| module.name() == "yama")
}

pub(crate) use self::task::{set_task_attr_current, task_attr_current, task_attrs_enabled};

pub(crate) fn securityfs_nodes() -> Vec<Arc<dyn SysObj>> {
    modules::active_modules()
        .iter()
        .filter_map(|module| module.securityfs_node())
        .collect()
}

pub(super) fn init() {
    for module in modules::active_modules() {
        info!("[kernel] LSM module enabled: {}", module.name());
    }
}

mod task;
