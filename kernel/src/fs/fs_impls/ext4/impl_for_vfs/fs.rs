// SPDX-License-Identifier: MPL-2.0

//! FileSystem adapter for Ext4.

use crate::{
    fs::{
        ext4::fs::Ext4,
        utils::NAME_MAX,
        vfs::{
            file_system::{FileSystem, FsEventSubscriberStats, FsFlags, SuperBlock},
            inode::Inode,
        },
    },
    prelude::*,
};

use super::super::super_block::MAGIC_NUM;

impl FileSystem for Ext4 {
    fn name(&self) -> &'static str {
        "ext4"
    }

    fn sync(&self) -> Result<()> {
        self.block_device().sync()?;
        Ok(())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root_inode().unwrap()
    }

    fn sb(&self) -> SuperBlock {
        let sb = self.super_block();
        SuperBlock {
            magic: MAGIC_NUM as _,
            bsize: sb.block_size(),
            blocks: sb.blocks_count() as _,
            bfree: sb.free_blocks_count() as _,
            bavail: sb.free_blocks_count() as _,
            files: sb.inodes_count() as _,
            ffree: sb.free_inodes_count() as _,
            fsid: 0,
            namelen: NAME_MAX,
            frsize: sb.block_size(),
            flags: 0,
            container_dev_id: self.container_device_id(),
        }
    }

    fn flags(&self) -> FsFlags {
        FsFlags::empty()
    }

    fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        self.fs_event_subscriber_stats()
    }
}
