// SPDX-License-Identifier: MPL-2.0

use super::super::modules;
use crate::{fs::vfs::path::Path, prelude::*, vm::perms::VmPerms};

/// Runs mmap and mprotect hooks for file-backed executable mappings.
pub fn on_mmap_file(context: &MmapFileContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.on_mmap_file(context)?;
    }

    Ok(())
}

/// The inputs for checking a file-backed mapping.
pub struct MmapFileContext<'a> {
    path: &'a Path,
    perms: VmPerms,
}

impl<'a> MmapFileContext<'a> {
    /// Creates an mmap file context.
    pub const fn new(path: &'a Path, perms: VmPerms) -> Self {
        Self { path, perms }
    }

    /// Returns the mapped file path.
    pub const fn path(&self) -> &'a Path {
        self.path
    }

    /// Returns the requested mapping permissions.
    pub const fn perms(&self) -> VmPerms {
        self.perms
    }
}
