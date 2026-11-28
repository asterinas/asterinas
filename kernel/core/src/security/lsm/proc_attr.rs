// SPDX-License-Identifier: MPL-2.0

//! Task attribute queries for procfs.
//!
//! Modules declare an optional [`CurrentAttrReader`] through [`super::LsmModule`].
//! Registered modules supply the directory names; only active modules answer reads.

use alloc::borrow::Cow;

use super::modules;
use crate::{prelude::*, process::posix_thread::PosixThread};

/// Reads a target task's current security attribute, including its trailing newline.
pub type CurrentAttrReader = fn(&PosixThread) -> Result<Cow<'static, str>>;

/// Lists registered modules with a procfs current attribute.
pub(crate) fn module_names() -> impl Iterator<Item = &'static str> {
    modules::registered_modules()
        .values()
        .filter(|module| module.current_attr_reader().is_some())
        .map(|module| module.name())
}

/// Reads from the named active module, or the first provider for the legacy path.
pub(crate) fn read_current(
    module_name: Option<&str>,
    target: &PosixThread,
) -> Result<Cow<'static, str>> {
    for module in modules::active_modules() {
        if module_name.is_some_and(|name| name != module.name()) {
            continue;
        }
        if let Some(read_fn) = module.current_attr_reader() {
            return read_fn(target);
        }
    }

    // Reference: <https://github.com/torvalds/linux/blob/v6.16/include/linux/lsm_hook_defs.h#L290>.
    Err(errno!(EINVAL, "no active LSM provides this task attribute"))
}
