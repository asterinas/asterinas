// SPDX-License-Identifier: MPL-2.0

//! Writeback and reclaim for EXT2/EXT4 inodes.
//!
//! This module contains the inode paths that make in-memory state durable and
//! reclaim deleted inodes. Writeback must keep xattrs, data pages, mapping
//! metadata, and inode-table state ordered consistently; reclaim must release
//! all storage owned by a zero-link inode exactly once.

#![short_vis_path::add(ext4)]

use super::{super::Ext4, Inode, InodeInner, RawInode};
use crate::fs::ext4::{prelude::*, utils};

impl Inode {
    /// Flushes all dirty state owned by the inode.
    ///
    /// Writes xattrs, data pages, and mapping nodes back before staging dirty
    /// inode metadata in the block group's inode-table page cache.
    pub(in ext4) fn sync_all(&self) -> Result<()> {
        if let Some(xattr) = &self.xattr {
            xattr.flush()?;
        }
        self.sync_data()
    }

    /// Flushes dirty file data and the metadata required to retrieve it.
    ///
    /// Writes data pages and mapping nodes back before staging dirty inode
    /// metadata in the block group's inode-table page cache. This does not flush
    /// xattrs.
    pub(in ext4) fn sync_data(&self) -> Result<()> {
        let fs = self.fs()?;
        let mut inner = self.inner.write();
        inner.sync_data_pages()?;
        inner.write_back_inode_desc(&fs, self.ino)
    }

    pub(super) fn defer_deleted_inode_work(&mut self) {
        let Ok(fs) = self.fs() else { return };
        let mut inner = self.inner.write();
        let mapping = inner.block_manager().ok().cloned();
        inner.set_dtime(utils::now());
        inner.set_file_size(0);
        let raw = RawInode::from(&*inner.desc);
        fs.defer_inode_reclaim(super::reclaim::PendingInodeWork {
            ino: self.ino,
            type_: self.type_,
            raw,
            mapping,
            xattr: self.xattr.take(),
        });
    }

    /// Attempts final reclaim for a deleted inode.
    pub(super) fn try_reclaim_deleted_inode(&self) -> Result<bool> {
        if self.link_count() != 0 {
            return Ok(false);
        }

        let fs = self.fs()?;
        let group = fs.block_group(self.block_group_idx);
        if !group.is_inode_allocated(self.ino) {
            return Ok(false);
        }

        if let Some(xattr) = self.xattr.as_ref() {
            xattr.delete_xattr_block()?;
        }

        let mut inner = self.inner.write();
        let old_size = inner.file_size();
        let block_manager = inner.block_manager().ok().cloned();
        if block_manager.is_some() {
            inner.resize_page_cache(0, old_size)?;
        }
        inner.set_dtime(utils::now());
        inner.set_file_size(0);
        inner.set_file_acl(0);
        // The descriptor's sector count may predate allocations made since
        // its last writeback. Reclaim through the authoritative mapping even
        // when that descriptor still reports an empty file.
        if let Some(block_manager) = block_manager {
            block_manager.truncate_to_byte_len(0)?;
        }
        inner.write_back_inode_desc(&fs, self.ino)?;

        fs.free_inode(self.ino, self.type_)?;
        Ok(true)
    }
}

impl InodeInner {
    /// Writes dirty mapping nodes before staging the inode descriptor in page cache.
    ///
    /// Also handles dirty external nodes when the inode-resident mapping is
    /// clean. A mapping writeback error leaves the descriptor unstaged and dirty
    /// state available for retry. Successful staging clears inode-local dirty
    /// flags; the block group remains responsible for flushing the inode table.
    pub(super) fn write_back_inode_desc(&mut self, fs: &Ext4, ino: Ext4Ino) -> Result<()> {
        if !self.is_dirty() {
            return Ok(());
        }

        self.sync_mapping_metadata()?;
        let raw_block_ptrs = self.raw_block_ptrs();
        self.desc.block_ptrs = raw_block_ptrs.block_ptrs;
        self.desc.sector_count = raw_block_ptrs.sector_count;

        let raw_inode = RawInode::from(&*self.desc);
        fs.write_back_inode_desc(ino, &raw_inode)?;
        self.clear_dirty();
        Ok(())
    }

    fn sync_data_pages(&self) -> Result<()> {
        let file_size = self.file_size();
        if file_size == 0 {
            return Ok(());
        }

        // Flush dirty data pages in [0, file_size) and wait for completion.
        match &self.payload {
            super::InodePayload::DataBacked { page_cache, .. } => {
                page_cache.flush_range(0..file_size)
            }
            _ => Ok(()),
        }
    }

    fn sync_mapping_metadata(&self) -> Result<()> {
        match &self.payload {
            super::InodePayload::DataBacked { block_manager, .. } => block_manager.sync_metadata(),
            _ => Ok(()),
        }
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use crate::fs::ext4::{
        FilePerm,
        fs::{MountFlavor, ROOT_INO},
        prelude::*,
        test_utils::{ExtFixtureBuilder, create_file},
    };

    #[ktest]
    fn deleted_inodes_without_mappings_retry_failed_reclaim() {
        crate::time::clocks::init_for_ktest();
        for flavor in [MountFlavor::Ext2, MountFlavor::Ext4] {
            let f = ExtFixtureBuilder::new(1, 256)
                .with_mount_flavor(flavor)
                .with_free_inodes(1000, 1000)
                .build()
                .unwrap();
            for inode_type in [
                InodeType::SymLink,
                InodeType::CharDevice,
                InodeType::BlockDevice,
                InodeType::NamedPipe,
                InodeType::Socket,
            ] {
                let before = f.fs.super_block().read_state().free_inodes_count();
                let inode =
                    f.fs.create_inode(ROOT_INO, inode_type, FilePerm::from_bits_truncate(0o600))
                        .unwrap();
                let ino = inode.ino;
                assert!(inode.inner.read().block_manager().is_err());
                inode.inner.write().desc.link_count = 0;
                // Make free_inode fail after descriptor staging. Drop must
                // transfer ownership of unfinished cleanup even without a tree.
                f.fs.super_block()
                    .write_state()
                    .set_free_inodes_count(u32::MAX);
                drop(inode);
                assert!(f.fs.block_group(0).is_inode_allocated(ino));
                let retry = f.fs.retry_deferred_reclaim();
                f.fs.super_block()
                    .write_state()
                    .set_free_inodes_count(before - 1);
                assert!(retry.is_err());
                f.fs.retry_deferred_reclaim().unwrap();
                assert!(!f.fs.block_group(0).is_inode_allocated(ino));
                assert_eq!(f.fs.super_block().read_state().free_inodes_count(), before);
                assert_eq!(f.fs.block_group(0).free_inodes_count(), 1000);
                f.fs.retry_deferred_reclaim().unwrap();
                assert_eq!(f.fs.super_block().read_state().free_inodes_count(), before);
            }
        }
    }

    #[ktest]
    fn extent_inode_publication_waits_for_nodes_and_drop_retries_reclaim() {
        crate::time::clocks::init_for_ktest();
        let f = ExtFixtureBuilder::new(1, 2048)
            .with_mount_flavor(MountFlavor::Ext4)
            .with_free_blocks(64, 64)
            .with_free_inodes(1000, 1000)
            .with_group0_used_dirs(1)
            .build()
            .unwrap();
        let file = create_file(&f.root(), "publish");
        let ino = file.ino;
        let mapping = file.inner.read().block_manager().unwrap().clone();
        for i in 0..5 {
            mapping.allocate_direct_blocks(i * 3, i * 3 + 1).unwrap();
        }
        let root = mapping.raw_block_ptrs().block_ptrs;
        let bid = root[4];
        f.disk.fail_write_block(Some(bid as usize));
        assert!(
            file.inner
                .write()
                .write_back_inode_desc(&f.fs, ino)
                .is_err()
        );
        assert!(mapping.has_dirty_metadata());
        let group = f.fs.block_group(0);
        group.sync_inode_table().unwrap();
        assert_ne!(
            group.read_inode_desc((ino - 1) as u16).unwrap().block_ptrs,
            root
        );
        f.disk.fail_write_block(None);
        file.inner
            .write()
            .write_back_inode_desc(&f.fs, ino)
            .unwrap();
        assert_eq!(
            group.read_inode_desc((ino - 1) as u16).unwrap().block_ptrs,
            root
        );
        assert!(!mapping.has_dirty_metadata());
        // Converting an external leaf changes neither the root nor sector count.
        // Both sync entry points must still flush it when the descriptor is clean.
        for logical in [0, 3] {
            mapping.mark_direct_written(logical, logical + 1).unwrap();
            assert!(!mapping.is_inode_dirty());
            assert!(!file.inner.read().desc.is_dirty());
            assert!(mapping.has_dirty_metadata());
            let writes = f.disk.block_writes(bid as usize);
            f.disk.fail_write_block(Some(bid as usize));
            let sync = || {
                if logical == 0 {
                    file.sync_data()
                } else {
                    file.sync_all()
                }
            };
            assert!(sync().is_err());
            assert!(mapping.has_dirty_metadata());
            f.disk.fail_write_block(None);
            sync().unwrap();
            assert_eq!(f.disk.block_writes(bid as usize), writes + 2);
            assert!(!mapping.has_dirty_metadata());
        }
        // Force post-accept cleanup failure on final Drop. Neither the inode
        // number nor unfinished block frees may be forgotten or reused early.
        let free = f.fs.super_block().read_state().free_blocks_count();
        f.fs.super_block()
            .write_state()
            .set_free_blocks_count(u32::MAX);
        file.inner.write().desc.link_count = 0;
        drop(group.remove_inode(ino));
        drop(file);
        assert!(group.is_inode_allocated(ino));
        f.fs.super_block().write_state().set_free_blocks_count(free);
        f.fs.retry_deferred_reclaim().unwrap();
        assert!(!group.is_inode_allocated(ino));
        assert_eq!(
            f.fs.super_block().read_state().free_blocks_count(),
            free + 6
        );
        f.fs.retry_deferred_reclaim().unwrap();
        assert_eq!(
            f.fs.super_block().read_state().free_blocks_count(),
            free + 6
        );
    }
}
