// SPDX-License-Identifier: MPL-2.0

use ostd::sync::RwMutexReadGuard;

use crate::{
    fs::{
        ext2::{utils::Dirty, Ext2, SuperBlock as Ext2SuperBlock, MAGIC_NUM as EXT2_MAGIC},
        utils::{FileSystem, FsFlags, Inode, SuperBlock, NAME_MAX},
    },
    prelude::*,
};

impl FileSystem for Ext2 {
    fn sync(&self) -> Result<()> {
        self.sync_all_inodes()?;
        self.sync_metadata()?;
        Ok(())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root_inode().unwrap()
    }

    fn sb(&self) -> SuperBlock {
        SuperBlock::from(self.super_block())
    }

    fn flags(&self) -> FsFlags {
        FsFlags::empty()
    }
}

impl From<RwMutexReadGuard<'_, Dirty<Ext2SuperBlock>>> for SuperBlock {
    fn from(ext2_sb: RwMutexReadGuard<Dirty<Ext2SuperBlock>>) -> Self {
        Self {
            magic: EXT2_MAGIC as _,
            bsize: ext2_sb.block_size(),
            blocks: ext2_sb.total_blocks() as _,
            bfree: ext2_sb.free_blocks_count() as _,
            bavail: ext2_sb.free_blocks_count() as _,
            files: ext2_sb.total_inodes() as _,
            ffree: ext2_sb.free_inodes_count() as _,
            fsid: 0, // TODO
            namelen: NAME_MAX,
            frsize: ext2_sb.fragment_size(),
            flags: 0, // TODO
        }
    }
}
