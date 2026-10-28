// SPDX-License-Identifier: MPL-2.0

//! Registration interface for immutable top-level securityfs directories.
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
    pub const fn read_only(name: &'static str, callback: ReadCallback) -> Self {
        Self {
            name,
            ops: FileOps::ReadOnly(callback),
        }
    }

    pub const fn write_only(name: &'static str, callback: WriteCallback) -> Self {
        Self {
            name,
            ops: FileOps::WriteOnly(callback),
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
            FileOps::ReadOnly(callback) => callback(offset, writer),
            FileOps::WriteOnly(_) => Err(Error::new(Errno::EBADF)),
        }
    }

    pub(crate) fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        match self.ops {
            FileOps::ReadOnly(_) => Err(Error::new(Errno::EBADF)),
            FileOps::WriteOnly(callback) => callback(offset, reader),
        }
    }
}

/// Registers an immutable top-level directory and its files in securityfs.
pub fn register_directory(name: &'static str, files: &'static [SecurityFsFile]) -> Result<()> {
    crate::fs::securityfs::register_directory(name, files)
}
