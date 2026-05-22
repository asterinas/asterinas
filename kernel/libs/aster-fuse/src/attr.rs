// SPDX-License-Identifier: MPL-2.0

use crate::{FuseGeneration, FuseNodeId};

/// FUSE inode attributes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct Attr {
    /// Inode number.
    ino: u64,
    /// File size in bytes.
    size: u64,
    /// Number of allocated blocks.
    blocks: u64,
    /// Last access time in seconds since the Unix epoch.
    atime: u64,
    /// Last modification time in seconds since the Unix epoch.
    mtime: u64,
    /// Last status-change time in seconds since the Unix epoch.
    ctime: u64,
    /// Nanosecond component of [`Attr::atime`].
    atimensec: u32,
    /// Nanosecond component of [`Attr::mtime`].
    mtimensec: u32,
    /// Nanosecond component of [`Attr::ctime`].
    ctimensec: u32,
    /// File type and permission bits.
    mode: u32,
    /// Number of hard links.
    nlink: u32,
    /// Owner user ID.
    uid: u32,
    /// Owner group ID.
    gid: u32,
    /// Device number for special files.
    rdev: u32,
    /// Preferred block size for I/O.
    blksize: u32,
    padding: u32,
}

impl Attr {
    /// Returns the inode number.
    pub fn ino(&self) -> u64 {
        self.ino
    }

    /// Returns the file size in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Returns the number of allocated blocks.
    pub fn blocks(&self) -> u64 {
        self.blocks
    }

    /// Returns the last access time in seconds since the Unix epoch.
    pub fn atime(&self) -> u64 {
        self.atime
    }

    /// Returns the last modification time in seconds since the Unix epoch.
    pub fn mtime(&self) -> u64 {
        self.mtime
    }

    /// Returns the last status-change time in seconds since the Unix epoch.
    pub fn ctime(&self) -> u64 {
        self.ctime
    }

    /// Returns the nanosecond component of the access time.
    pub fn atimensec(&self) -> u32 {
        self.atimensec
    }

    /// Returns the nanosecond component of the modification time.
    pub fn mtimensec(&self) -> u32 {
        self.mtimensec
    }

    /// Returns the nanosecond component of the status-change time.
    pub fn ctimensec(&self) -> u32 {
        self.ctimensec
    }

    /// Returns the file type and permission bits.
    pub fn mode(&self) -> u32 {
        self.mode
    }

    /// Returns the number of hard links.
    pub fn nlink(&self) -> u32 {
        self.nlink
    }

    /// Returns the owner user ID.
    pub fn uid(&self) -> u32 {
        self.uid
    }

    /// Returns the owner group ID.
    pub fn gid(&self) -> u32 {
        self.gid
    }

    /// Returns the device number for special files.
    pub fn rdev(&self) -> u32 {
        self.rdev
    }

    /// Returns the preferred block size for I/O.
    pub fn blksize(&self) -> u32 {
        self.blksize
    }
}

/// The reply payload for lookup-like FUSE operations.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct EntryReply {
    /// Node ID assigned to the resolved inode.
    nodeid: FuseNodeId,
    /// Generation number used to distinguish reused inode numbers.
    generation: FuseGeneration,
    /// Entry-cache timeout in seconds.
    entry_valid: u64,
    /// Attribute-cache timeout in seconds.
    attr_valid: u64,
    /// Nanosecond component of [`EntryReply::entry_valid`].
    entry_valid_nsec: u32,
    /// Nanosecond component of [`EntryReply::attr_valid`].
    attr_valid_nsec: u32,
    /// Attributes of the resolved inode.
    attr: Attr,
}

impl EntryReply {
    /// Creates an `EntryReply` from the resolved inode data.
    pub const fn new(
        nodeid: FuseNodeId,
        generation: FuseGeneration,
        entry_valid: u64,
        attr_valid: u64,
        entry_valid_nsec: u32,
        attr_valid_nsec: u32,
        attr: Attr,
    ) -> Self {
        Self {
            nodeid,
            generation,
            entry_valid,
            attr_valid,
            entry_valid_nsec,
            attr_valid_nsec,
            attr,
        }
    }

    /// Returns the resolved inode's node ID.
    pub fn nodeid(&self) -> FuseNodeId {
        self.nodeid
    }

    /// Returns the generation number.
    pub fn generation(&self) -> FuseGeneration {
        self.generation
    }

    /// Returns the entry-cache timeout in seconds.
    pub fn entry_valid(&self) -> u64 {
        self.entry_valid
    }

    /// Returns the attribute-cache timeout in seconds.
    pub fn attr_valid(&self) -> u64 {
        self.attr_valid
    }

    /// Returns the nanosecond component of the entry-cache timeout.
    pub fn entry_valid_nsec(&self) -> u32 {
        self.entry_valid_nsec
    }

    /// Returns the nanosecond component of the attribute-cache timeout.
    pub fn attr_valid_nsec(&self) -> u32 {
        self.attr_valid_nsec
    }

    /// Returns the resolved inode attributes.
    pub fn attr(&self) -> Attr {
        self.attr
    }
}
