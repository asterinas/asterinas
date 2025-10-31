// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::AtomicU32;

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;

use super::Inode;
use crate::prelude::*;

#[derive(Debug, Clone)]
pub struct SuperBlock {
    pub magic: u64,
    pub bsize: usize,
    pub blocks: usize,
    pub bfree: usize,
    pub bavail: usize,
    pub files: usize,
    pub ffree: usize,
    pub fsid: u64,
    pub namelen: usize,
    pub frsize: usize,
    pub flags: u64,
}

impl SuperBlock {
    pub fn new(magic: u64, block_size: usize, name_max_len: usize) -> Self {
        Self {
            magic,
            bsize: block_size,
            blocks: 0,
            bfree: 0,
            bavail: 0,
            files: 0,
            ffree: 0,
            fsid: 0,
            namelen: name_max_len,
            frsize: block_size,
            flags: 0,
        }
    }
}

bitflags! {
    /// Flags for per file system.
    pub struct FsFlags: u32 {
        /// The filesystem is mounted read-only.
        const RDONLY        =   1 << 0;
        /// Writes are synced at once.
        const SYNCHRONOUS   =   1 << 4;
        /// Allow mandatory locks on an FS.
        const MANDLOCK      =   1 << 6;
        /// Directory modifications are synchronous.
        const DIRSYNC       =   1 << 7;
        /// Suppress certain messages in kernel log.
        const SILENT        =   1 << 15;
        /// Update the on-disk [acm]times lazily.
        const LAZYTIME      =   1 << 25;
    }
}

impl core::fmt::Display for FsFlags {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        if self.contains(FsFlags::RDONLY) {
            write!(f, "ro")?;
        } else {
            write!(f, "rw")?;
        }
        if self.contains(FsFlags::SYNCHRONOUS) {
            write!(f, ",sync")?;
        }
        if self.contains(FsFlags::MANDLOCK) {
            write!(f, ",mandlock")?;
        }
        if self.contains(FsFlags::DIRSYNC) {
            write!(f, ",dirsync")?;
        }
        if self.contains(FsFlags::SILENT) {
            write!(f, ",silent")?;
        }
        if self.contains(FsFlags::LAZYTIME) {
            write!(f, ",lazytime")?;
        }
        Ok(())
    }
}

impl From<u32> for FsFlags {
    fn from(value: u32) -> Self {
        Self::from_bits_truncate(value)
    }
}

impl From<FsFlags> for u32 {
    fn from(value: FsFlags) -> Self {
        value.bits()
    }
}

define_atomic_version_of_integer_like_type!(FsFlags, {
    /// An atomic version of `FsFlags`.
    #[derive(Debug)]
    #[expect(dead_code)]
    pub struct AtomicFsFlags(AtomicU32);
});

pub trait FileSystem: Any + Sync + Send {
    /// Gets the name of this FS type such as `"ext4"` or `"sysfs"`.
    fn name(&self) -> &'static str;

    /// Syncs the file system.
    fn sync(&self) -> Result<()>;

    /// Returns the root inode of this file system.
    fn root_inode(&self) -> Arc<dyn Inode>;

    /// Returns the super block of this file system.
    fn sb(&self) -> SuperBlock;

    /// Returns the flags of this file system.
    fn flags(&self) -> FsFlags {
        // TODO: Currently we do not support any flags for filesystems.
        // Remove the default empty implementation in the future.
        FsFlags::empty()
    }

    /// Sets the flags of this file system.
    fn set_fs_flags(&self, _flags: FsFlags, _data: Option<CString>, _ctx: &Context) -> Result<()> {
        // TODO: Remove the default empty implementation in the future.
        warn!("setting file system flags is not implemented");
        Ok(())
    }
}

impl dyn FileSystem {
    pub fn downcast_ref<T: FileSystem>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref::<T>()
    }
}

impl Debug for dyn FileSystem {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("FileSystem")
            .field("super_block", &self.sb())
            .field("flags", &self.flags())
            .finish()
    }
}
