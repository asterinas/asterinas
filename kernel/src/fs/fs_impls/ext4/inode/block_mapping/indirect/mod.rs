// SPDX-License-Identifier: MPL-2.0

//! Ext2-style indirect block-mapping engine.
//!
//! It maps a file's logical blocks to physical device blocks through the
//! classic direct/indirect block-pointer tree rooted in the inode's
//! `i_block`. This is the mapping used by inodes without the `EXTENTS` flag,
//! i.e. every inode of an ext2-format volume.

mod block_ptr_tree;
mod indirect_block;

use core::sync::atomic::{AtomicUsize, Ordering};

use self::block_ptr_tree::{BlockPtrTree, RawBlockPtrs, ResolvedBlockRange};
use super::{
    super::{
        super::{fs::Ext4, prelude::*},
        RAW_BLOCK_PTRS_LEN,
    },
    MapState, Mapping,
};

/// On-disk width of an ext2-style block pointer.
///
/// The pointer-tree format stores every block number in 32 bits; conversions
/// to and from the module-wide `Ext4Bid` happen at the engine boundary.
pub(super) type IndirectBid = u32;

/// Bridges the inode's logical file view and the physical block device.
///
/// `IndirectManager` maintains the relationship between logical file blocks,
/// physical block addresses, and the page-cache view of file contents. Sparse
/// logical ranges are represented by absent (zero) block pointers, while
/// allocated ranges must remain consistent with the inode's block-pointer
/// tree.
#[derive(Debug)]
pub(in super::super) struct IndirectManager {
    /// Translates logical file block indices to physical device block
    /// addresses and manages block allocation and truncation.
    block_ptr_tree: RwMutex<BlockPtrTree>,
    /// Cached `npages` bound for `PageCache`.
    npages: AtomicUsize,
    /// File system handle for indirect I/O and BIO submission.
    fs: Weak<Ext4>,
}

impl IndirectManager {
    /// Creates the engine from the inode's on-disk `i_block` and `i_blocks`.
    pub(super) fn new(
        root: [u32; RAW_BLOCK_PTRS_LEN],
        sector_count: u64,
        fs: Weak<Ext4>,
        npages: usize,
    ) -> Result<Self> {
        // Non-extent inodes store `i_blocks` with 32-bit on-disk accounting;
        // a larger value cannot come from a valid volume.
        let sector_count = u32::try_from(sector_count).map_err(|_| {
            Error::with_message(
                Errno::EIO,
                "inode sector count exceeds the 32-bit format limit",
            )
        })?;
        let tree = BlockPtrTree::new(RawBlockPtrs::new(sector_count, root), fs.clone());
        Ok(Self {
            block_ptr_tree: RwMutex::new(tree),
            npages: AtomicUsize::new(npages),
            fs,
        })
    }

    /// Returns a strong reference to the owning filesystem.
    fn fs(&self) -> Result<Arc<Ext4>> {
        self.fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "filesystem dropped"))
    }

    /// Maps logical block `iblock` to a physical run.
    ///
    /// Allocated runs extend to the end of the contiguous physical range
    /// (bounded by the leaf-pointer boundary); holes report a conservative
    /// length that never crosses a possibly allocated pointer. The indirect
    /// format has no unwritten state, so allocated runs always read as
    /// written.
    pub(super) fn map_blocks(&self, iblock: Iblock) -> Result<Mapping> {
        let tree = self.block_ptr_tree.read();
        let range = tree.lookup_block_range(iblock, u32::MAX)?;
        if !range.is_empty() {
            let len = u32::try_from(range.len()).expect("leaf-bounded run fits u32");
            return Ok(Mapping::Mapped {
                pblock: Ext4Bid::from(range.start),
                len,
                state: MapState::Written,
            });
        }
        // Both lookups run under the same tree read guard, so the hole
        // observed above cannot be filled in between; `max(1)` keeps the
        // reported run non-empty regardless.
        let len = tree.approx_hole_blocks(iblock, u32::MAX)?.max(1);
        Ok(Mapping::Hole { len })
    }

    /// Looks up a single logical block -> physical block.
    fn lookup_block(&self, iblock: Iblock) -> Result<Option<IndirectBid>> {
        let tree = self.block_ptr_tree.read();
        tree.lookup_block(iblock)
    }

    /// Returns a copy of the inode's 60-byte `i_block` (the pointer array).
    pub(super) fn root_snapshot(&self) -> [u32; RAW_BLOCK_PTRS_LEN] {
        self.block_ptr_tree.read().raw_block_ptrs().block_ptrs
    }

    /// Returns the inode's `i_blocks` (512-byte sectors) accounting.
    pub(super) fn sector_count(&self) -> u64 {
        u64::from(self.block_ptr_tree.read().raw_block_ptrs().sector_count)
    }

    /// Returns whether the block-pointer tree has uncommitted changes.
    pub(super) fn is_dirty(&self) -> bool {
        self.block_ptr_tree.read().is_dirty()
    }

    /// Clears the block-pointer dirty flag after writeback.
    pub(super) fn clear_dirty(&self) {
        self.block_ptr_tree.write().clear_dirty();
    }

    /// Truncates blocks to the new byte length (best-effort).
    ///
    /// Errors are logged, not propagated: the inode reclaim path relies on
    /// truncation never failing, and blocks leaked by a partial failure are
    /// recoverable by e2fsck. Linux ext2 follows the same practice.
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

impl BlockAsPageCacheBackend for IndirectManager {
    fn submit_read_bio(
        &self,
        idx: usize,
        bio_segment: BioSegment,
        complete_fn: BioCompleteFn,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        if idx >= self.npages.load(Ordering::Acquire) {
            return_errno_with_message!(Errno::EINVAL, "read past end of inode");
        }
        let iblock = Iblock::try_from(idx)
            .map_err(|_| Error::with_message(Errno::EINVAL, "logical block number overflow"))?;
        match self.lookup_block(iblock)? {
            Some(bid) => {
                let fs = self.fs()?;
                fs.read_blocks_async(Ext4Bid::from(bid), bio_segment, Some(complete_fn), io_batch)
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
            return_errno_with_message!(Errno::EINVAL, "write past end of inode");
        }
        let iblock = Iblock::try_from(idx)
            .map_err(|_| Error::with_message(Errno::EINVAL, "logical block number overflow"))?;
        let fs = self.fs()?;

        // Buffered writes pre-allocate in `prepare_write`, so the block is
        // usually already mapped; write it directly.
        if let Some(bid) = self.lookup_block(iblock)? {
            return fs.write_blocks_async(
                Ext4Bid::from(bid),
                bio_segment,
                Some(complete_fn),
                io_batch,
            );
        }

        // Encountered a hole; allocate a block. Since we dropped the read
        // lock above, another thread may have filled the hole; the `Existing`
        // arm below handles that race.
        let mut tree = self.block_ptr_tree.write();
        let step = tree.resolve_block_range(&fs, iblock, bio_segment.nblocks() as u32)?;
        let bid = match step {
            ResolvedBlockRange::NewlyAllocated(r) => r.start,
            ResolvedBlockRange::Existing(r) => r.start,
        };

        fs.write_blocks_async(Ext4Bid::from(bid), bio_segment, Some(complete_fn), io_batch)
    }
}

/// Returns the largest byte size an indirect-mapped file can reach.
///
/// Mirrors Linux's ext2 limit: the pointer-tree geometry bounds the logical
/// block count, and the 32-bit `i_blocks` accounting (512-byte sectors, data
/// plus indirect metadata) bounds the total block usage; the result is
/// additionally capped at `i64::MAX` bytes. For 4 KiB blocks this evaluates
/// to 2,196,873,666,560 bytes.
pub(super) fn max_file_size() -> usize {
    let max_bytes = (max_blocks() << BLOCK_SIZE.trailing_zeros()).min(i64::MAX as u64);
    usize::try_from(max_bytes).expect("Asterinas supports 64-bit architectures")
}

const fn max_blocks() -> u64 {
    const DIRECT_BLOCKS: u64 = 12;

    let block_size_bits = BLOCK_SIZE.trailing_zeros();
    let ptrs_per_block = 1u64 << (block_size_bits - 2);
    let mut max_blocks = DIRECT_BLOCKS;
    let mut metadata_blocks = 1u64;
    let mut upper_limit = (1u64 << 32) - 1;

    // `i_blocks` stores the total 512-byte sector count for data and
    // indirect metadata. Linux keeps ext2 one filesystem block below the
    // 32-bit sector accounting limit.
    upper_limit >>= block_size_bits - 9;

    max_blocks += 1u64 << (block_size_bits - 2);
    max_blocks += 1u64 << (2 * (block_size_bits - 2));
    max_blocks += 1u64 << (3 * (block_size_bits - 2));

    metadata_blocks += 1 + ptrs_per_block;
    metadata_blocks += 1 + ptrs_per_block + ptrs_per_block * ptrs_per_block;

    if max_blocks + metadata_blocks > upper_limit {
        max_blocks = upper_limit;

        upper_limit -= DIRECT_BLOCKS;

        metadata_blocks = 1;
        upper_limit -= ptrs_per_block;

        if upper_limit < ptrs_per_block * ptrs_per_block {
            metadata_blocks += 1 + upper_limit.div_ceil(ptrs_per_block);
            max_blocks -= metadata_blocks;
        } else {
            metadata_blocks += 1 + ptrs_per_block;
            upper_limit -= ptrs_per_block * ptrs_per_block;
            metadata_blocks += 1
                + upper_limit.div_ceil(ptrs_per_block)
                + upper_limit.div_ceil(ptrs_per_block * ptrs_per_block);
            max_blocks -= metadata_blocks;
        }
    }
    max_blocks
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    use super::*;

    #[ktest]
    fn max_file_size_matches_ext2_4k_limit() {
        assert_eq!(max_file_size(), 2_196_873_666_560);
    }
}
