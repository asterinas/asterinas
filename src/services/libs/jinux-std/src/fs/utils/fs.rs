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

pub trait FileSystem: Sync + Send {
    fn sync(&self) -> Result<()>;

    fn root_inode(&self) -> Arc<dyn Inode>;

    fn sb(&self) -> SuperBlock;
}
