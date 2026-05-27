// SPDX-License-Identifier: MPL-2.0

use super::super::modules;
use crate::{
    fs::{
        file::{AccessMode, StatusFlags},
        vfs::path::Path,
    },
    prelude::*,
};

/// Runs file open hooks in module order.
pub fn on_file_open(context: &FileOpenContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.on_file_open(context)?;
    }

    Ok(())
}

/// The inputs for checking a newly opened file.
pub struct FileOpenContext<'a> {
    path: &'a Path,
    access_mode: AccessMode,
    status_flags: StatusFlags,
}

impl<'a> FileOpenContext<'a> {
    /// Creates a file open context.
    pub const fn new(path: &'a Path, access_mode: AccessMode, status_flags: StatusFlags) -> Self {
        Self {
            path,
            access_mode,
            status_flags,
        }
    }

    /// Returns the path being opened.
    pub const fn path(&self) -> &'a Path {
        self.path
    }

    /// Returns the requested access mode.
    pub const fn access_mode(&self) -> AccessMode {
        self.access_mode
    }

    /// Returns the open status flags.
    pub const fn status_flags(&self) -> StatusFlags {
        self.status_flags
    }
}
