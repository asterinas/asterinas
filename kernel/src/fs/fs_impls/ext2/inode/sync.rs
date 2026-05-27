// SPDX-License-Identifier: MPL-2.0

//! Writeback and reclaim for ext2 inodes.
//!
//! This module contains the inode paths that make in-memory state durable and
//! reclaim deleted inodes. Writeback must keep xattrs, data pages, indirect
//! metadata, and inode-table state ordered consistently; reclaim must release
//! all storage owned by a zero-link inode exactly once.

use super::{super::Ext2, Inode, InodeInner, RawInode};
use crate::fs::ext2::{prelude::*, utils};

impl Inode {
    /// Flushes xattr, data pages, and indirect blocks, then writes the inode descriptor back to
    /// the block group's inode table page cache.
    ///
    /// Returns `true` if the descriptor was dirty and written back. The caller is responsible for
    /// flushing the inode table and the block device.
    pub(in crate::fs::fs_impls::ext2) fn sync_all(&self) -> Result<bool> {
        // `fsync` step 1: flush the xattr.
        if let Some(xattr) = &self.xattr {
            xattr.flush()?;
        }

        // `fsync` step 2: flush dirty data pages.
        let fs = self.fs()?;
        let mut inner = self.inner.write();
        inner.sync_data_pages()?;

        // `fsync` step 3: flush inode-local indirect metadata before
        // persisting inode-table state.
        inner.sync_indirect_blocks()?;

        // `fsync` step 4: persist inode metadata to the inode table page cache.
        let desc_is_dirty = if inner.is_dirty() {
            inner.write_back_desc(&fs, self.ino)?;
            true
        } else {
            false
        };

        Ok(desc_is_dirty)
    }

    /// Flushes dirty data pages, indirect blocks, and dirty inode metadata.
    ///
    /// Returns `true` if the descriptor was dirty and written back. The caller is responsible for
    /// flushing the inode table and the block device.
    pub(in crate::fs::fs_impls::ext2) fn sync_data(&self) -> Result<bool> {
        let fs = self.fs()?;
        let mut inner = self.inner.write();

        // `fdatasync` writes back dirty data pages first. The caller is
        // responsible for the final device-cache flush.
        inner.sync_data_pages()?;

        // `fdatasync` must also persist dirty indirect metadata needed to reach
        // newly written data blocks before the final device flush.
        inner.sync_indirect_blocks()?;

        // Persist metadata conservatively whenever the descriptor is dirty so
        // `fdatasync` does not miss file-size or block-mapping updates.
        let desc_is_dirty = if inner.is_dirty() {
            inner.write_back_desc(&fs, self.ino)?;
            true
        } else {
            false
        };
        Ok(desc_is_dirty)
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
        if inner.desc.sector_count > 0
            && let Some(block_manager) = block_manager
        {
            block_manager.truncate_to_byte_len(0);
        }
        inner.write_back_desc(&fs, self.ino)?;

        fs.free_inode(self.ino, self.type_)?;
        Ok(true)
    }
}

impl InodeInner {
    /// Serializes the descriptor to disk and clears the dirty flag.
    pub(super) fn write_back_desc(&mut self, fs: &Ext2, ino: Ext2Ino) -> Result<()> {
        let raw_block_ptrs = self.raw_block_ptrs();
        self.desc.block_ptrs = raw_block_ptrs.block_ptrs;
        self.desc.sector_count = raw_block_ptrs.sector_count;
        let raw_inode = RawInode::from(&*self.desc);
        fs.write_inode_desc(ino, &raw_inode)?;
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

    fn sync_indirect_blocks(&self) -> Result<()> {
        match &self.payload {
            super::InodePayload::DataBacked { block_manager, .. } => {
                block_manager.sync_indirect_blocks()
            }
            _ => Ok(()),
        }
    }
}
