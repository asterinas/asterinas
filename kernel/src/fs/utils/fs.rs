// SPDX-License-Identifier: MPL-2.0

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
    pub struct FsFlags: u32 {
        /// Dentry cannot be evicted.
        const DENTRY_UNEVICTABLE = 1 << 1;
    }
}

bitflags! {
    /// Mount options for a filesystem.
    pub struct FsMountOptions: u8 {
        /// The filesystem is mounted read-only.
        const READONLY      =   1 << 0;
        /// Writes are synced at once.
        const SYNCHRONOUS   =   1 << 1;
        /// Allow mandatory locks on an FS.
        const MANDLOCK      =   1 << 2;
        /// Directory modifications are synchronous.
        const DIRSYNC       =   1 << 3;
        /// Suppress certain messages in kernel log.
        const SILENT        =   1 << 4;
        /// Update the on-disk [acm]times lazily.
        const LAZYTIME      =   1 << 5;
    }
}

pub trait FileSystem: Any + Sync + Send {
    fn sync(&self) -> Result<()>;

    fn root_inode(&self) -> Arc<dyn Inode>;

    fn sb(&self) -> SuperBlock;

    fn flags(&self) -> FsFlags;

    fn set_mount_options(
        &self,
        _options: FsMountOptions,
        _data: Vaddr,
        _ctx: &Context,
    ) -> Result<()> {
        // TODO: Currently we do not support any flags for filesystems.
        // Remove the default empty implementation and add handling code
        // for each filesystem in the future.
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
