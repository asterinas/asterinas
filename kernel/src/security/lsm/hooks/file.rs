// SPDX-License-Identifier: MPL-2.0

use super::super::modules;
use crate::{
    fs::{file::Permission, vfs::path::Path},
    prelude::*,
};

/// Runs file operation permission hooks in module order.
pub fn on_file_permission(context: &FilePermissionContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.on_file_permission(context)?;
    }

    Ok(())
}

/// The inputs for checking an operation on an opened file.
pub struct FilePermissionContext<'a> {
    path: &'a Path,
    permission: Permission,
}

impl<'a> FilePermissionContext<'a> {
    /// Creates a file permission context.
    pub const fn new(path: &'a Path, permission: Permission) -> Self {
        Self { path, permission }
    }

    /// Returns the path backing the opened file.
    pub const fn path(&self) -> &'a Path {
        self.path
    }

    /// Returns the requested permission.
    pub const fn permission(&self) -> Permission {
        self.permission
    }
}
