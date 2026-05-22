// SPDX-License-Identifier: MPL-2.0

use alloc::string::String;

use int_to_c_enum::TryFromInt;

/// A raw directory entry in a `FUSE_READDIR` reply.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct Dirent {
    /// Inode number of the directory entry.
    ino: u64,
    /// Offset cookie for continuing directory iteration.
    off: u64,
    /// Length of the entry name in bytes.
    namelen: u32,
    /// POSIX directory entry type.
    typ: u32,
}

impl Dirent {
    /// Returns the inode number of the directory entry.
    pub fn ino(&self) -> u64 {
        self.ino
    }

    /// Returns the offset cookie for continuing directory iteration.
    pub fn off(&self) -> u64 {
        self.off
    }

    /// Returns the length of the entry name in bytes.
    pub fn namelen(&self) -> u32 {
        self.namelen
    }

    /// Returns the POSIX directory entry type.
    pub fn typ(&self) -> u32 {
        self.typ
    }
}

/// An opaque offset cookie for continuing directory iteration.
///
/// The offset cookie may differ between different servers.
/// The FUSE protocol treats it as an opaque value, and the client should not interpret its value.
//
// FIXME: we currently treat the offset as a simple byte offset in the directory stream,
// but some servers may use a different format for the offset cookie.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirOffset(u64);

impl DirOffset {
    /// Creates an offset cookie from its wire value.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the wire value of this offset cookie.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A directory entry decoded from a `FUSE_READDIR` reply.
#[derive(Debug, Clone)]
pub struct FuseDirEntry {
    ino: u64,
    offset: DirOffset,
    type_: DirentType,
    name: String,
}

impl FuseDirEntry {
    pub fn new(ino: u64, offset: DirOffset, type_: DirentType, name: String) -> Self {
        Self {
            ino,
            offset,
            type_,
            name,
        }
    }

    /// Returns the inode number of this directory entry.
    pub fn ino(&self) -> u64 {
        self.ino
    }

    /// Returns the offset cookie for continuing directory iteration.
    pub fn offset(&self) -> DirOffset {
        self.offset
    }

    /// Returns the POSIX directory entry type.
    pub fn type_(&self) -> DirentType {
        self.type_
    }

    /// Returns the entry name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// POSIX `d_type` values carried in [`Dirent::typ`].
///
/// See <https://www.man7.org/linux/man-pages/man3/readdir.3.html>.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub enum DirentType {
    Unknown = 0,
    Fifo = 1,
    Char = 2,
    Dir = 4,
    Block = 6,
    Regular = 8,
    Link = 10,
    Sock = 12,
}
