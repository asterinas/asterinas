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

pub trait FileSystem: Any + Sync + Send {
    fn sync(&self) -> Result<()>;

    fn root_inode(&self) -> Arc<dyn Inode>;

    fn sb(&self) -> SuperBlock;

    fn flags(&self) -> FsFlags;
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
