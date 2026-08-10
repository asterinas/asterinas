// SPDX-License-Identifier: MPL-2.0

//! Minimal task-centered AppArmor integration.
//!
//! [`AppArmorLsm`] participates in the common LSM interfaces for regular-file open checks, task attributes, and securityfs controls.
//! The `label`, `policy`, `file`, and `securityfs` submodules own those AppArmor-specific concerns.

mod file;
mod label;
mod policy;
mod securityfs;

use alloc::format;

use aster_systree::SysObj;

pub(in crate::security::lsm) use self::label::Label;
use super::super::{
    LsmFlags, LsmModule, LsmTaskAttrs,
    hooks::{FileOpenContext, LsmFileOpenHook},
};
use crate::{prelude::*, process::posix_thread::PosixThread};

pub(super) static APPARMOR_LSM: AppArmorLsm = AppArmorLsm;
pub(super) const UNCONFINED_PROFILE_NAME: &str = "unconfined";

/// The AppArmor major LSM.
pub(super) struct AppArmorLsm;

impl LsmModule for AppArmorLsm {
    fn name(&self) -> &'static str {
        "apparmor"
    }

    fn flags(&self) -> LsmFlags {
        LsmFlags::LEGACY_MAJOR | LsmFlags::EXCLUSIVE
    }

    fn file_open_hook(&self) -> Option<&dyn LsmFileOpenHook> {
        Some(self)
    }

    fn task_attrs(&self) -> Option<&dyn LsmTaskAttrs> {
        Some(self)
    }

    fn securityfs_node(&self) -> Option<Arc<dyn SysObj>> {
        Some(securityfs::new_node())
    }
}

impl LsmTaskAttrs for AppArmorLsm {
    fn current(&self, posix_thread: &PosixThread) -> Result<String> {
        let value = label::task_profile_name(posix_thread)
            .map(|profile_name| format!("{} (enforce)", profile_name))
            .unwrap_or_else(|| UNCONFINED_PROFILE_NAME.to_string());
        Ok(value)
    }

    fn set_current(&self, posix_thread: &PosixThread, value: &str) -> Result<()> {
        label::confine_task(posix_thread, value)
    }
}

impl LsmFileOpenHook for AppArmorLsm {
    fn on_file_open(&self, context: &FileOpenContext<'_>) -> Result<()> {
        file::open(context)
    }
}
