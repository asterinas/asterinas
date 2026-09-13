// SPDX-License-Identifier: MPL-2.0

//! Per-inode logical-to-physical block mapping.
//!
//! [`BlockMapping`] is the single owner of mapping synchronization, page-count
//! bounds, and the filesystem reference. The format-specific manager stores
//! only the on-disk mapping state. Callers therefore use one interface whether
//! an inode uses classic block pointers or another EXT mapping format.
//!
//! Mapping locks protect only in-memory mapping lookup and mutation. They must
//! not be held while submitting data BIOs. The inode lock remains responsible
//! for coordinating mapping changes with inode size and metadata updates.

mod block_manager;

use core::sync::atomic::{AtomicUsize, Ordering};

use aster_block::bio::BioCompleteFn;

pub(super) use self::block_manager::RawBlockPtrs;
use self::block_manager::{BlockPtrTree, ResolvedBlockRange};
use super::{Iblock, io_range::IoRangeIter};
use crate::fs::ext4::{fs::Ext4, prelude::*};

/// A data-backed inode's logical-to-physical block mapping.
#[derive(Debug)]
pub(super) struct BlockMapping {
    manager: RwMutex<InodeBlockManager>,
    npages: AtomicUsize,
    fs: Weak<Ext4>,
}

/// Format-specific mutable mapping state.
#[derive(Debug)]
pub(super) enum InodeBlockManager {
    /// A classic EXT direct/indirect block-pointer tree.
    BlockPtrTree(BlockPtrTree),
}

impl BlockMapping {
    /// Creates an indirect mapping from the inode's pointer array.
    pub(super) fn new_indirect(
        raw_block_ptrs: RawBlockPtrs,
        fs: Weak<Ext4>,
        npages: usize,
    ) -> Self {
        Self {
            manager: RwMutex::new(InodeBlockManager::BlockPtrTree(BlockPtrTree::new(
                raw_block_ptrs,
                fs.clone(),
            ))),
            npages: AtomicUsize::new(npages),
            fs,
        }
    }

    /// Returns the mapping root and sector count to store in the raw inode.
    pub(super) fn raw_block_ptrs(&self) -> RawBlockPtrs {
        let manager = self.manager.read();
        match &*manager {
            InodeBlockManager::BlockPtrTree(tree) => *tree.raw_block_ptrs(),
        }
    }

    /// Returns whether the in-inode mapping representation needs writeback.
    pub(super) fn is_dirty(&self) -> bool {
        let manager = self.manager.read();
        match &*manager {
            InodeBlockManager::BlockPtrTree(tree) => tree.is_dirty(),
        }
    }

    /// Marks the in-inode mapping representation as written back.
    pub(super) fn clear_dirty(&self) {
        let mut manager = self.manager.write();
        match &mut *manager {
            InodeBlockManager::BlockPtrTree(tree) => tree.clear_dirty(),
        }
    }

    /// Returns mapped physical runs and holes over a logical block range.
    pub(super) fn iter_io_ranges(&self, block_range: Range<Iblock>) -> IoRangeIter<'_> {
        IoRangeIter::new_indirect(block_range, self.manager.read())
    }

    /// Removes mappings beyond the block containing `new_size`.
    pub(super) fn truncate_to_byte_len(&self, new_size: usize) {
        let fs = match self.fs() {
            Ok(fs) => fs,
            Err(err) => {
                error!("truncate: failed to get fs reference, err: {:?}", err);
                return;
            }
        };
        let mut manager = self.manager.write();
        match &mut *manager {
            InodeBlockManager::BlockPtrTree(tree) => tree.truncate_to_byte_len(&fs, new_size),
        }
    }

    /// Writes mapping metadata stored outside the inode to the block device.
    pub(super) fn sync_metadata(&self) -> Result<()> {
        let manager = self.manager.read();
        match &*manager {
            InodeBlockManager::BlockPtrTree(tree) => tree.sync_indirect_blocks(),
        }
    }

    /// Ensures that every logical block in `[start_block, end_block)` is mapped.
    pub(super) fn allocate_range_blocks(&self, start_block: usize, end_block: usize) -> Result<()> {
        let fs = self.fs()?;
        let mut manager = self.manager.write();
        let InodeBlockManager::BlockPtrTree(tree) = &mut *manager;
        let mut current_block = start_block;
        while current_block < end_block {
            let iblock = Iblock::try_from(current_block)
                .map_err(|_| Error::with_message(Errno::EINVAL, "logical block number overflow"))?;
            let remaining = u32::try_from(end_block - current_block)
                .map_err(|_| Error::with_message(Errno::EINVAL, "block range length overflow"))?;
            let range = match tree.resolve_block_range(&fs, iblock, remaining)? {
                ResolvedBlockRange::Existing(range) | ResolvedBlockRange::NewlyAllocated(range) => {
                    range
                }
            };
            debug_assert!(!range.is_empty());
            current_block += range.len();
        }
        Ok(())
    }

    /// Updates the page-count bound enforced by page-cache I/O submission.
    pub(super) fn set_npages(&self, npages: usize) {
        self.npages.store(npages, Ordering::Release);
    }

    fn fs(&self) -> Result<Arc<Ext4>> {
        self.fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "filesystem already dropped"))
    }

    fn lookup_block(&self, iblock: Iblock) -> Result<Option<Ext4Bid>> {
        let manager = self.manager.read();
        match &*manager {
            InodeBlockManager::BlockPtrTree(tree) => tree.lookup_block(iblock),
        }
    }
}

impl BlockAsPageCacheBackend for BlockMapping {
    fn submit_read_bio(
        &self,
        idx: usize,
        bio_segment: BioSegment,
        complete_fn: BioCompleteFn,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        if idx >= self.npages.load(Ordering::Acquire) {
            return_errno_with_message!(Errno::EINVAL, "invalid read size");
        }
        let iblock = Iblock::try_from(idx)
            .map_err(|_| Error::with_message(Errno::EINVAL, "logical block number overflow"))?;
        match self.lookup_block(iblock)? {
            Some(bid) => {
                let fs = self.fs()?;
                fs.read_blocks_async(bid, bio_segment, Some(complete_fn), io_batch)
            }
            None => {
                complete_fn(BioStatus::Zeros);
                Ok(())
            }
        }
    }

    fn submit_write_bio(
        &self,
        idx: usize,
        bio_segment: BioSegment,
        complete_fn: BioCompleteFn,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        if idx >= self.npages.load(Ordering::Acquire) {
            return_errno_with_message!(Errno::EINVAL, "invalid write size");
        }
        let iblock = Iblock::try_from(idx)
            .map_err(|_| Error::with_message(Errno::EINVAL, "logical block number overflow"))?;
        let fs = self.fs()?;

        if let Some(bid) = self.lookup_block(iblock)? {
            return fs.write_blocks_async(bid, bio_segment, Some(complete_fn), io_batch);
        }

        let bid = {
            let mut manager = self.manager.write();
            let InodeBlockManager::BlockPtrTree(tree) = &mut *manager;
            match tree.resolve_block_range(&fs, iblock, bio_segment.nblocks() as u32)? {
                ResolvedBlockRange::NewlyAllocated(range) | ResolvedBlockRange::Existing(range) => {
                    range.start
                }
            }
        };

        fs.write_blocks_async(bid, bio_segment, Some(complete_fn), io_batch)
    }
}
