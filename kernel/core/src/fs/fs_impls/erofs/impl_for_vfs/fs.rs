// SPDX-License-Identifier: MPL-2.0

//! VFS filesystem trait implementation for the EROFS instance.

use device_id::DeviceId;

use super::super::{fs::Erofs, ondisk::super_block};
use crate::{
    fs::{
        utils,
        vfs::{
            file_system::{FileSystem, FsEventSubscriberStats, FsFlags, SuperBlock},
            inode::Inode,
        },
    },
    prelude::*,
};

impl Erofs {
    /// Prepares the immutable VFS statistics before filesystem publication.
    pub(in super::super) fn prepare_vfs_super_block(
        checked: &super_block::SuperBlock,
        device_id: DeviceId,
    ) -> ostd::Result<SuperBlock> {
        let mut super_block = SuperBlock::new(
            u64::from(super_block::EROFS_SUPER_MAGIC_V1),
            checked.block_size_bytes(),
            utils::NAME_MAX,
            device_id,
        );
        super_block.blocks =
            usize::try_from(checked.reported_blocks()).map_err(|_| ostd::Error::Overflow)?;
        let used_inodes =
            usize::try_from(checked.reported_inodes()).map_err(|_| ostd::Error::Overflow)?;
        // Linux statfs reports maximal inode capacity, not writable space.
        super_block.files = usize::MAX;
        super_block.ffree = usize::MAX - used_inodes;
        super_block.flags = u64::from(FsFlags::RDONLY.bits());
        Ok(super_block)
    }
}

impl FileSystem for Erofs {
    fn name(&self) -> &'static str {
        "erofs"
    }

    fn sync(&self) -> Result<()> {
        Ok(())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root_inode()
    }

    fn sb(&self) -> SuperBlock {
        self.vfs_super_block().clone()
    }

    fn flags(&self) -> FsFlags {
        self.fs_flags()
    }

    fn set_fs_flags(&self, flags: FsFlags, data: Option<&str>, _ctx: &Context) -> Result<()> {
        if !flags.contains(FsFlags::RDONLY) {
            return Err(Error::new(Errno::EROFS));
        }
        if flags != self.flags() || data.is_some() {
            return Err(Error::new(Errno::EOPNOTSUPP));
        }
        Ok(())
    }

    fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        self.fs_event_subscriber_stats()
    }
}
