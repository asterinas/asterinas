// SPDX-License-Identifier: MPL-2.0

//! Registration interface for immutable securityfs hierarchies.
//!
//! [`SecurityFsFile`] describes one read-only or write-only file backed by an
//! operation callback. The VFS offset is passed through unchanged, callbacks
//! return the number of bytes transferred, and callback errors retain their
//! original errno. This seam has no per-open authorization hook; consumers
//! that require authorization must check it inside every I/O callback.

use aster_systree::SysPerms;

use crate::prelude::*;

pub type ReadCallback = fn(usize, &mut VmWriter) -> Result<usize>;
pub type WriteCallback = fn(usize, &mut VmReader) -> Result<usize>;

#[derive(Clone, Copy, Debug)]
enum FileOps {
    ReadOnly(ReadCallback),
    WriteOnly(WriteCallback),
}

/// A file exposed by a top-level securityfs directory.
#[derive(Clone, Copy, Debug)]
pub struct SecurityFsFile {
    name: &'static str,
    ops: FileOps,
}

impl SecurityFsFile {
    pub const fn read_only(name: &'static str, read_fn: ReadCallback) -> Self {
        Self {
            name,
            ops: FileOps::ReadOnly(read_fn),
        }
    }

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

/// An immutable securityfs directory declaration.
#[derive(Clone, Copy, Debug)]
pub struct SecurityFsDirectory {
    name: &'static str,
    files: &'static [SecurityFsFile],
    directories: &'static [SecurityFsDirectory],
}

impl SecurityFsDirectory {
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

/// Registers an immutable top-level directory and its files in securityfs.
pub fn register_directory(
    name: &'static str,
    files: &'static [SecurityFsFile],
) -> Result<()> {
    crate::fs::securityfs::register_directory(&SecurityFsDirectory::new(name, files, &[]))
}

/// Registers an immutable directory hierarchy in securityfs.
pub fn register_hierarchy(directory: &'static SecurityFsDirectory) -> Result<()> {
    crate::fs::securityfs::register_directory(directory)
}
