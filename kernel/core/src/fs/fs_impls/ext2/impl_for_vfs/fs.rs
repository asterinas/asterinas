// SPDX-License-Identifier: MPL-2.0

//! [`FileSystem`] trait implementation for [`Ext2`].
//!
//! Translates VFS-level mount, sync, stat, and root-inode requests into
//! the corresponding ext2-internal operations.

use aster_block::{BLOCK_SIZE, bio::BioStatus};

use crate::{
    fs::{
        fs_impls::ext2::{Ext2, super_block::MAGIC_NUM},
        utils::NAME_MAX,
        vfs::{
            file_system::{FileSystem, FsEventSubscriberStats, SuperBlock},
            inode::Inode,
        },
    },
    prelude::*,
};

impl FileSystem for Ext2 {
    fn name(&self) -> &'static str {
        "ext2"
    }

    fn sync(&self) -> Result<()> {
        self.sync_all()?;
        if self.block_device().sync()? != BioStatus::Complete {
            return_errno_with_message!(Errno::EIO, "failed to flush block device");
        }
        Ok(())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root_inode().unwrap()
    }

    fn sb(&self) -> SuperBlock {
        let sb = self.super_block();
        let blocks = if self.uses_minix_df() {
            sb.total_blocks()
        } else {
            sb.total_blocks().saturating_sub(sb.total_metadata_blocks())
        };
        SuperBlock {
            magic: MAGIC_NUM as u64,
            bsize: BLOCK_SIZE,
            blocks: blocks as usize,
            bfree: sb.free_blocks_count() as usize,
            bavail: sb
                .free_blocks_count()
                .saturating_sub(sb.reserved_blocks_count()) as usize,
            files: sb.total_inodes() as usize,
            ffree: sb.free_inodes_count() as usize,
            fsid: 0,
            namelen: NAME_MAX,
            frsize: sb.fragment_size(),
            flags: 0,
            container_dev_id: self.container_device_id(),
        }
    }

    fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        self.fs_event_subscriber_stats()
    }
}
