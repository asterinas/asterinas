// SPDX-License-Identifier: MPL-2.0

//! LSM hook inputs for opening files.

use super::super::modules;
use crate::{
    fs::{
        file::{AccessMode, CreationFlags, StatusFlags},
        vfs::path::{AbsPathResult, Path, PathResolver},
    },
    prelude::*,
    process::posix_thread::PosixThread,
};

bitflags! {
    /// Semantic permissions requested by a file-open operation.
    pub struct FileOpenAccess: u8 {
        /// Opens the file for reading.
        const READ = 1 << 0;
        /// Opens the file for writing.
        const WRITE = 1 << 1;
        /// Truncates an existing regular file.
        const TRUNCATE = 1 << 2;
    }
}

/// Runs file-open hooks in module order.
pub fn on_file_open(context: &FileOpenContext<'_>) -> Result<()> {
    for hook in modules::active_modules()
        .iter()
        .filter_map(|module| module.file_open_hook())
    {
        hook.on_file_open(context)?;
    }

    Ok(())
}

/// The inputs for checking a file-open operation.
pub struct FileOpenContext<'a> {
    path: &'a Path,
    path_resolver: &'a PathResolver,
    posix_thread: &'a PosixThread,
    is_regular_file: bool,
    requested_access: FileOpenAccess,
}

impl<'a> FileOpenContext<'a> {
    /// Creates a context for opening a path.
    pub(crate) fn new(
        path: &'a Path,
        path_resolver: &'a PathResolver,
        posix_thread: &'a PosixThread,
        access_mode: AccessMode,
        creation_flags: CreationFlags,
        status_flags: StatusFlags,
    ) -> Self {
        let is_regular_file = path.inode().type_().is_regular_file();
        Self {
            path,
            path_resolver,
            posix_thread,
            is_regular_file,
            requested_access: requested_access(
                access_mode,
                creation_flags,
                status_flags,
                is_regular_file,
            ),
        }
    }

    /// Returns the thread requesting the file handle.
    pub const fn posix_thread(&self) -> &PosixThread {
        self.posix_thread
    }

    /// Returns whether the target is a regular file.
    pub const fn is_regular_file(&self) -> bool {
        self.is_regular_file
    }

    /// Returns the semantic permissions requested by the operation.
    pub const fn requested_access(&self) -> FileOpenAccess {
        self.requested_access
    }

    /// Resolves the target pathname from the requesting thread's root.
    pub fn resolve_path_name(&self) -> AbsPathResult {
        self.path_resolver.make_abs_path(self.path)
    }
}

fn requested_access(
    access_mode: AccessMode,
    creation_flags: CreationFlags,
    status_flags: StatusFlags,
    is_regular_file: bool,
) -> FileOpenAccess {
    if status_flags.contains(StatusFlags::O_PATH) {
        return FileOpenAccess::empty();
    }

    let mut requested = FileOpenAccess::empty();
    if access_mode.is_readable() {
        requested.insert(FileOpenAccess::READ);
    }
    if access_mode.is_writable() {
        requested.insert(FileOpenAccess::WRITE);
    }
    if is_regular_file && creation_flags.contains(CreationFlags::O_TRUNC) {
        requested.insert(FileOpenAccess::TRUNCATE);
    }

    requested
}
