use crate::fs::utils::{FileSystem, FsFlags, Inode, PageCache, SuperBlock, NAME_MAX};
use crate::prelude::*;

use ext2::{Ext2, Ext2SuperBlock};

impl FileSystem for Ext2 {
    fn sync(&self) -> Result<()> {
        self.sync_inodes()?;
        self.sync_metadata()?;
        Ok(())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root_inode::<PageCache>().unwrap()
    }

    fn sb(&self) -> SuperBlock {
        SuperBlock::from(self.super_block())
    }

    fn flags(&self) -> FsFlags {
        FsFlags::empty()
    }
}

impl From<Ext2SuperBlock> for SuperBlock {
    fn from(ext2_sb: Ext2SuperBlock) -> Self {
        Self {
            magic: ext2_sb.magic as _,
            bsize: ext2_sb.block_size(),
            blocks: ext2_sb.blocks_count as _,
            bfree: ext2_sb.free_blocks_count as _,
            bavail: ext2_sb.free_blocks_count as _,
            files: ext2_sb.inodes_count as _,
            ffree: ext2_sb.free_inodes_count as _,
            fsid: 0, // TODO
            namelen: NAME_MAX,
            frsize: ext2_sb.fragment_size(),
            flags: 0, // TODO
        }
    }
}
