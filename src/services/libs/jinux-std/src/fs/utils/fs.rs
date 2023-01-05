use alloc::sync::Arc;

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

pub trait FileSystem: Sync + Send {
    fn sync(&self) -> Result<()>;

    fn root_inode(&self) -> Arc<dyn Inode>;

    fn sb(&self) -> SuperBlock;
}
