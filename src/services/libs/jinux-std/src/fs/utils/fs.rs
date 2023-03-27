use alloc::sync::Arc;
use bitflags::bitflags;

use super::Inode;
use crate::prelude::*;

#[derive(Debug, Clone)]
pub struct SuperBlock {
    pub magic: usize,
    pub bsize: usize,
    pub blocks: usize,
    pub bfree: usize,
    pub bavail: usize,
    pub files: usize,
    pub ffree: usize,
    pub fsid: usize,
    pub namelen: usize,
    pub frsize: usize,
    pub flags: usize,
}

impl SuperBlock {
    pub fn new(magic: usize, block_size: usize, name_len: usize) -> Self {
        Self {
            magic,
            bsize: block_size,
            blocks: 0,
            bfree: 0,
            bavail: 0,
            files: 0,
            ffree: 0,
            fsid: 0,
            namelen: 255,
            frsize: block_size,
            flags: 0,
        }
    }
}

bitflags! {
    pub struct FsFlags: u32 {
        /// Disable page cache.
        const NO_PAGECACHE = 1 << 0;
        /// Dentry cannot be evicted.
        const DENTRY_UNEVICTABLE = 1 << 1;
    }
}

pub trait FileSystem: Sync + Send {
    fn sync(&self) -> Result<()>;

    fn root_inode(&self) -> Arc<dyn Inode>;

    fn sb(&self) -> SuperBlock;

    fn flags(&self) -> FsFlags;
}
