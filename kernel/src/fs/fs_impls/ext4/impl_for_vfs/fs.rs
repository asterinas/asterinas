// SPDX-License-Identifier: MPL-2.0

//! `FileSystem` trait implementation for `Ext4`.

use aster_block::BLOCK_SIZE;

use crate::{
    fs::{
        fs_impls::ext4::{Ext4, super_block::MAGIC_NUM},
        utils::NAME_MAX,
        vfs::{
            file_system::{FileSystem, FsEventSubscriberStats, FsFlags, SuperBlock},
            inode::Inode,
        },
    },
    prelude::*,
};

impl FileSystem for Ext4 {
    fn name(&self) -> &'static str {
        "ext4"
    }

    fn sync(&self) -> Result<()> {
        // Read-only mount: there is nothing to flush — no dirty inodes, no
        // journal, no block-side metadata mutations — so sync(2) is a no-op.
        Ok(())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root_inode().unwrap()
    }

    fn flags(&self) -> FsFlags {
        // Read-only mount: the VFS refuses directory-entry mutations
        // (create/unlink/rename/...) with `EROFS` via `check_mount_writable`.
        FsFlags::RDONLY
    }

    fn sb(&self) -> SuperBlock {
        let sb = self.super_block();
        // `f_blocks` reports usable capacity, not the raw device size: the total
        // block count minus the filesystem's metadata overhead — superblock/GDT
        // copies, bitmaps, inode tables (`SuperBlock::metadata_overhead`) —
        // matching Linux `ext4_statfs` (`ext4_blocks_count - s_overhead`). The
        // journal's log blocks would add to this overhead, but this read-only
        // mount consults no journal, so only the static metadata overhead counts.
        let overhead = sb.metadata_overhead();
        let usable_blocks = sb.total_blocks().saturating_sub(overhead);
        // `bavail` excludes the root-reserved blocks (`s_r_blocks_count`), like
        // Linux `ext4_statfs`, so unprivileged `df` sees the space it can use.
        let bavail = sb
            .free_blocks_count()
            .saturating_sub(u64::from(sb.reserved_blocks_count()));
        SuperBlock {
            magic: MAGIC_NUM as u64,
            bsize: BLOCK_SIZE,
            blocks: usize::try_from(usable_blocks).unwrap(),
            bfree: usize::try_from(sb.free_blocks_count()).unwrap(),
            bavail: usize::try_from(bavail).unwrap(),
            files: sb.total_inodes() as usize,
            ffree: sb.free_inodes_count() as usize,
            // The volume UUID's low 64 bits, so mounts are distinguishable
            // (Linux folds the UUID into `f_fsid` too).
            fsid: u64::from_le_bytes(sb.uuid()[..8].try_into().unwrap()),
            namelen: NAME_MAX,
            frsize: BLOCK_SIZE,
            flags: FsFlags::RDONLY.bits() as u64,
            container_dev_id: self.container_device_id(),
        }
    }

    fn set_fs_flags(&self, _flags: FsFlags, _data: Option<CString>, _ctx: &Context) -> Result<()> {
        // Refuse loudly instead of inheriting the VFS default, which logs a
        // warning and returns `Ok(())` — a fake success that would pretend to
        // honor a flag change this mount never performs. Ext4 here accepts no
        // runtime filesystem-flag change, so any change is `EOPNOTSUPP`.
        self.refuse_fs_flags_change()
    }

    fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        self.fs_event_subscriber_stats()
    }
}
