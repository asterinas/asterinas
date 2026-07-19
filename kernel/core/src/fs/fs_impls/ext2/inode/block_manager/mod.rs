// SPDX-License-Identifier: MPL-2.0

//! Physical-block lifecycle management for a single ext2 inode.

mod block_ptr_tree;
mod indirect_block_manager;

use core::sync::atomic::{AtomicUsize, Ordering};

use aster_block::bio::BioCompleteFn;

use self::block_ptr_tree::ResolvedBlockRange;
pub(super) use self::block_ptr_tree::{BlockPtrTree, RawBlockPtrs};
use super::io_range::IoRangeIter;
use crate::fs::ext2::{fs::Ext2, prelude::*};

/// Bridges the inode's logical file view and the physical block device.
///
/// `InodeBlockManager` maintains the relationship between logical file
/// blocks, ext2 physical block addresses, and the page-cache view of file
/// contents. Sparse logical ranges are represented by absent block mappings,
/// while allocated ranges must remain consistent with the inode's
/// block-pointer tree.
#[derive(Debug)]
pub(super) struct InodeBlockManager {
    /// Translates logical file block indices to physical device block addresses and
    /// manages block allocation and truncation.
    block_ptr_tree: RwMutex<BlockPtrTree>,
    /// Cached `npages` bound for `PageCache`.
    npages: AtomicUsize,
    /// File system handle for indirect I/O and BIO submission.
    fs: Weak<Ext2>,
}

impl InodeBlockManager {
    /// Creates a new block manager wrapping the given block-pointer tree.
    pub(super) fn new(block_ptr_tree: BlockPtrTree, fs: Weak<Ext2>, npages: usize) -> Self {
        Self {
            block_ptr_tree: RwMutex::new(block_ptr_tree),
            npages: AtomicUsize::new(npages),
            fs,
        }
    }

    /// Returns a strong reference to the owning filesystem.
    pub(super) fn fs(&self) -> Result<Arc<Ext2>> {
        self.fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "filesystem already dropped"))
    }

    /// Looks up a single logical block -> physical block.
    pub(super) fn lookup_block(&self, iblock: Iblock) -> Result<Option<Ext2Bid>> {
        let tree = self.block_ptr_tree.read();
        tree.lookup_block(iblock)
    }

    /// Returns a snapshot of the raw block pointer state.
    pub(super) fn raw_block_ptrs(&self) -> RawBlockPtrs {
        *self.block_ptr_tree.read().raw_block_ptrs()
    }

    /// Returns whether the block-pointer tree has uncommitted changes.
    pub(super) fn is_dirty(&self) -> bool {
        self.block_ptr_tree.read().is_dirty()
    }

    /// Clears the block-pointer dirty flag after writeback.
    pub(super) fn clear_dirty(&self) {
        self.block_ptr_tree.write().clear_dirty();
    }

    /// Creates an iterator over existing and hole block ranges.
    ///
    /// The returned iterator holds a read lock on the block-pointer tree for
    /// its entire lifetime. Callers should consume it promptly to avoid
    /// blocking concurrent allocations or truncations on this inode.
    pub(super) fn iter_io_ranges(&self, block_range: Range<Iblock>) -> IoRangeIter<'_> {
        let tree = self.block_ptr_tree.read();
        IoRangeIter::new(block_range, tree)
    }

    /// Truncates blocks to the new_size (best-effort).
    pub(super) fn truncate_to_byte_len(&self, new_size: usize) {
        let fs = match self.fs() {
            Ok(fs) => fs,
            Err(err) => {
                error!("truncate: failed to get fs reference, err: {:?}", err);
                return;
            }
        };
        let mut tree = self.block_ptr_tree.write();
        tree.truncate_to_byte_len(&fs, new_size)
    }

    /// Flushes all dirty cached indirect blocks to the device.
    pub(super) fn sync_indirect_blocks(&self) -> Result<()> {
        self.block_ptr_tree.write().sync_indirect_blocks()
    }

    /// Allocates missing data blocks that cover the requested logical block range.
    pub(super) fn allocate_range_blocks(&self, start_block: usize, end_block: usize) -> Result<()> {
        let fs = self.fs()?;
        let mut tree = self.block_ptr_tree.write();
        let mut current_block = start_block;
        while current_block < end_block {
            let iblock = Iblock::try_from(current_block)
                .map_err(|_| Error::with_message(Errno::EINVAL, "logical block number overflow"))?;
            let remaining = u32::try_from(end_block - current_block)
                .map_err(|_| Error::with_message(Errno::EINVAL, "block range length overflow"))?;

            let block_range = tree.resolve_block_range(&fs, iblock, remaining)?;
            match block_range {
                ResolvedBlockRange::Existing(range) => {
                    debug_assert!(!range.is_empty());
                    current_block += range.len();
                }
                ResolvedBlockRange::NewlyAllocated(range) => {
                    debug_assert!(!range.is_empty());
                    current_block += range.len();
                }
            }
        }
        Ok(())
    }

    /// Updates the cached page-cache capacity bound.
    pub(super) fn set_npages(&self, npages: usize) {
        self.npages.store(npages, Ordering::Release);
    }
}

impl BlockAsPageCacheBackend for InodeBlockManager {
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
                // Encountered a hole, zero fill the page.
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

        // TODO: Refactor `lookup_block` and `resolve_block_range`. Currently
        // `bio_segment.nblocks()` is always 1, so the point lookup works correctly, but the
        // semantics are misleading because `bio_segment` can represent a contiguous block range.
        // The block is already allocated; write it directly.
        if let Some(bid) = self.lookup_block(iblock)? {
            return fs.write_blocks_async(bid, bio_segment, Some(complete_fn), io_batch);
        }

        // Encounter a hole; allocate a block. Since we dropped the read lock
        // above, another thread may have filled the hole; the `Existing` arm
        // below handles that race.
        let mut tree = self.block_ptr_tree.write();
        let step = tree.resolve_block_range(&fs, iblock, bio_segment.nblocks() as u32)?;
        let bid = match step {
            ResolvedBlockRange::NewlyAllocated(r) => r.start,
            ResolvedBlockRange::Existing(r) => r.start,
        };

        fs.write_blocks_async(bid, bio_segment, Some(complete_fn), io_batch)
    }
}
