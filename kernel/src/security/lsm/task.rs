// SPDX-License-Identifier: MPL-2.0

//! Process attributes supplied by active LSM modules.

use super::modules;
use crate::{prelude::*, process::posix_thread::PosixThread};

pub(crate) fn task_attrs_enabled() -> bool {
    modules::active_modules()
        .iter()
        .any(|module| module.task_attrs().is_some())
}

pub(crate) fn task_attr_current(posix_thread: &PosixThread) -> Result<String> {
    modules::active_modules()
        .iter()
        .find_map(|module| module.task_attrs())
        .ok_or_else(|| Error::with_message(Errno::ENOENT, "no LSM task attribute is available"))?
        .current(posix_thread)
}

pub(crate) fn set_task_attr_current(posix_thread: &PosixThread, value: &str) -> Result<()> {
    modules::active_modules()
        .iter()
        .find_map(|module| module.task_attrs())
        .ok_or_else(|| Error::with_message(Errno::ENOENT, "no LSM task attribute is available"))?
        .set_current(posix_thread, value)
}
