// SPDX-License-Identifier: MPL-2.0

//! [`FileSystem`] implementation for [`F2fs`].

use super::super::{
    fs::F2fs,
    ondisk::{F2FS_BLKSIZE, F2FS_SUPER_MAGIC},
    prelude::*,
};
use crate::fs::vfs::{
    file_system::{FileSystem, FsEventSubscriberStats, FsFlags, SuperBlock},
    inode::Inode,
};

/// The maximum file name length supported by F2FS.
const F2FS_NAME_LEN: usize = 255;

impl FileSystem for F2fs {
    fn name(&self) -> &'static str {
        "f2fs"
    }

    fn sync(&self) -> Result<()> {
        self.flush_checkpoint()
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        F2fs::root_inode(self).unwrap()
    }

    fn sb(&self) -> SuperBlock {
        let blocks = self.user_block_count();
        let free_blocks = self.free_blocks().min(blocks);
        SuperBlock {
            magic: F2FS_SUPER_MAGIC as u64,
            bsize: F2FS_BLKSIZE,
            blocks: self.user_block_count(),
            bfree: free_blocks,
            bavail: free_blocks,
            files: self.max_inodes(),
            ffree: 0,
            fsid: 0,
            namelen: F2FS_NAME_LEN,
            frsize: F2FS_BLKSIZE,
            flags: self.fs_flags().bits() as u64,
            container_dev_id: self.container_device_id(),
        }
    }

    fn flags(&self) -> FsFlags {
        self.fs_flags()
    }

    fn set_fs_flags(&self, flags: FsFlags, _data: Option<&str>, _ctx: &Context) -> Result<()> {
        self.set_fs_flags(flags);
        Ok(())
    }

    fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        self.fs_event_subscriber_stats()
    }
}
