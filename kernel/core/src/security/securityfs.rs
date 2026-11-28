// SPDX-License-Identifier: MPL-2.0

//! Registration interface for securityfs hierarchies.
//!
//! [`SecurityFsDirectory`] groups callback-backed files and child directories.
//! Additional [`SysObj`] children can expose directories whose contents change
//! at runtime.
//!
//! File permissions do not replace module-specific authorization. Callbacks that
//! require authorization must check the current caller on each operation,
//! including reads through an already open file descriptor.

use aster_systree::{SysObj, SysPerms};

use crate::prelude::*;

/// A read operation receiving the file offset and destination buffer.
///
/// Returns the number of bytes copied, or an error passed unchanged to the caller.
pub type ReadCallback = fn(usize, &mut VmWriter) -> Result<usize>;
/// A write operation receiving the file offset and source buffer.
///
/// Returns the number of bytes consumed, or an error passed unchanged to the caller.
pub type WriteCallback = fn(usize, &mut VmReader) -> Result<usize>;

#[derive(Clone, Copy, Debug)]
enum FileOps {
    ReadOnly(ReadCallback),
    WriteOnly(WriteCallback),
}

/// A callback-backed file in a securityfs directory.
#[derive(Clone, Copy, Debug)]
pub struct SecurityFsFile {
    name: &'static str,
    ops: FileOps,
}

impl SecurityFsFile {
    /// Creates a read-only file with mode `0444`.
    pub const fn read_only(name: &'static str, read_fn: ReadCallback) -> Self {
        Self {
            name,
            ops: FileOps::ReadOnly(read_fn),
        }
    }

    /// Creates a write-only file with mode `0666`.
    ///
    /// The callback must enforce any module-specific access restrictions.
    pub const fn write_only(name: &'static str, write_fn: WriteCallback) -> Self {
        Self {
            name,
            ops: FileOps::WriteOnly(write_fn),
        }
    }

    pub(crate) const fn name(&self) -> &'static str {
        self.name
    }

    pub(crate) const fn perms(&self) -> SysPerms {
        match self.ops {
            FileOps::ReadOnly(_) => SysPerms::DEFAULT_RO_ATTR_PERMS,
            FileOps::WriteOnly(_) => SysPerms::from_bits_retain(0o666),
        }
    }

    pub(crate) fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        match self.ops {
            FileOps::ReadOnly(read_fn) => read_fn(offset, writer),
            FileOps::WriteOnly(_) => Err(Error::new(Errno::EBADF)),
        }
    }

    pub(crate) fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        match self.ops {
            FileOps::ReadOnly(_) => Err(Error::new(Errno::EBADF)),
            FileOps::WriteOnly(write_fn) => write_fn(offset, reader),
        }
    }
}

/// A securityfs directory declaration.
#[derive(Clone, Copy, Debug)]
pub struct SecurityFsDirectory {
    name: &'static str,
    files: &'static [SecurityFsFile],
    directories: &'static [SecurityFsDirectory],
}

impl SecurityFsDirectory {
    /// Declares a directory whose names are validated during registration.
    pub const fn new(
        name: &'static str,
        files: &'static [SecurityFsFile],
        directories: &'static [SecurityFsDirectory],
    ) -> Self {
        Self {
            name,
            files,
            directories,
        }
    }

    pub(crate) const fn name(&self) -> &'static str {
        self.name
    }

    pub(crate) const fn files(&self) -> &'static [SecurityFsFile] {
        self.files
    }

    pub(crate) const fn directories(&self) -> &'static [SecurityFsDirectory] {
        self.directories
    }
}

/// Registers a directory hierarchy and kernel-owned child nodes in securityfs.
///
/// Child nodes may expose dynamic state through the SysTree interfaces. Their
/// names must not collide with the declared files or directories.
/// Each declared directory must contain at least one declared file or directory;
/// the additional child nodes do not count toward this requirement.
///
/// Names must be nonempty, at most 255 bytes, and contain neither `/` nor NUL.
/// `.` and `..` are also invalid. Invalid declarations return `EINVAL`, and
/// duplicate names within a directory return `EEXIST`.
pub fn register_hierarchy(
    directory: &'static SecurityFsDirectory,
    children: &[Arc<dyn SysObj>],
) -> Result<()> {
    crate::fs::securityfs::register_directory(directory, children)
}
