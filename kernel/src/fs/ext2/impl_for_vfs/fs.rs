// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        ext2::{Ext2, MAGIC_NUM as EXT2_MAGIC},
        utils::{FileSystem, FsEventSubscriberStats, Inode, NAME_MAX, SuperBlock},
    },
    prelude::*,
};

impl FileSystem for Ext2 {
    fn name(&self) -> &'static str {
        "ext2"
    }

    fn sync(&self) -> Result<()> {
        self.sync_all_inodes()?;
        self.sync_metadata()?;

        self.block_device().sync()?;
        Ok(())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root_inode().unwrap()
    }

    fn sb(&self) -> SuperBlock {
        let ext2_sb = self.super_block();
        SuperBlock {
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
            container_dev_id: self.container_device_id(),
        }
    }

    fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        self.fs_event_subscriber_stats()
    }
}
