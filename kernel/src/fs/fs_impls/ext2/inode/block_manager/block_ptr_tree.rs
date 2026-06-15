// SPDX-License-Identifier: MPL-2.0

//! Logical-to-physical block translation via the ext2 block-pointer tree.

use device_id::{decode_device_numbers, encode_device_numbers};
use ostd::mm::io::util::HasVmReaderWriter;
use smallvec::SmallVec;

use super::indirect_block_manager::{IndirectBlock, IndirectBlockManager};
use crate::fs::ext2::{fs::Ext2, inode::RAW_BLOCK_PTRS_LEN, prelude::*};

const PTRS_PER_BLOCK: usize = BLOCK_SIZE / size_of::<u32>();
const SECTORS_PER_BLOCK: u32 = (BLOCK_SIZE / SECTOR_SIZE) as u32;
const MAX_BLOCK_POINTER_LEVELS: usize = 4;

/// An ext2 inode's block-pointer tree.
///
/// Each tree is rooted in the 15 raw block pointer slots stored in the inode.
/// The first 12 slots address data blocks directly,
/// while the remaining slots address progressively deeper indirect levels.
///
/// | Slot index | Region           | Addressable blocks (4 KiB blocks)     |
/// |------------|------------------|---------------------------------------|
/// | 0 – 11     | Direct           | 12 blocks                             |
/// | 12         | Single-indirect  | 1 024 blocks                          |
/// | 13         | Double-indirect  | 1 024² blocks                         |
/// | 14         | Triple-indirect  | 1 024³ blocks                         |
///
/// A zero pointer at any level represents an unallocated sparse range.
/// Non-zero pointers must refer either to data blocks at the leaf level
/// or to indirect metadata blocks at the level selected by the slot.
#[derive(Debug)]
pub(in crate::fs::fs_impls::ext2::inode) struct BlockPtrTree {
    raw_block_ptrs: Dirty<RawBlockPtrs>,
    indirect_blocks_manager: Mutex<IndirectBlockManager>,
}

impl BlockPtrTree {
    /// Creates a new block-pointer tree from the on-disk raw pointers.
    pub(in crate::fs::fs_impls::ext2::inode) fn new(
        raw_block_ptrs: RawBlockPtrs,
        fs: Weak<Ext2>,
    ) -> Self {
        Self {
            raw_block_ptrs: Dirty::new(raw_block_ptrs),
            indirect_blocks_manager: Mutex::new(IndirectBlockManager::new(fs)),
        }
    }

    /// Returns a reference to the raw on-disk block pointer state.
    pub(in crate::fs::fs_impls::ext2::inode) fn raw_block_ptrs(&self) -> &RawBlockPtrs {
        &self.raw_block_ptrs
    }

    /// Returns whether the `RawBlockPtrs` is dirty.
    pub(super) fn is_dirty(&self) -> bool {
        self.raw_block_ptrs.is_dirty()
    }

    /// Clears the dirty flag for the raw on-disk block pointer state.
    pub(super) fn clear_dirty(&mut self) {
        self.raw_block_ptrs.clear_dirty();
    }

    /// Flushes all dirty cached indirect blocks to the device.
    pub(in crate::fs::fs_impls::ext2::inode) fn sync_indirect_blocks(&self) -> Result<()> {
        self.indirect_blocks_manager.lock().sync()
    }

    /// Resolves a logical block to a contiguous physical block range.
    pub(in crate::fs::fs_impls::ext2::inode) fn lookup_block_range(
        &self,
        iblock: Iblock,
        max_blocks: u32,
    ) -> Result<Range<Ext2Bid>> {
        if max_blocks == 0 {
            return_errno_with_message!(Errno::EINVAL, "zero block range requested");
        }

        let walk = self.walk_at(iblock)?;
        self.existing_contiguous_data_blocks(&walk, max_blocks)
    }

    /// Resolves a logical block to physical block (read-only).
    pub(in crate::fs::fs_impls::ext2::inode) fn lookup_block(
        &self,
        iblock: Iblock,
    ) -> Result<Option<Ext2Bid>> {
        let range = self.lookup_block_range(iblock, 1)?;
        Ok(if range.is_empty() {
            None
        } else {
            Some(range.start)
        })
    }

    /// Resolves a logical block to a contiguous physical block range,
    /// allocating new blocks if the mapping does not yet exist.
    ///
    /// The returned range starts at the physical block backing `iblock` and
    /// extends for at most `max_blocks` contiguous physical blocks, but may
    /// be shorter when the contiguous run ends or a leaf-pointer boundary is
    /// reached.
    ///
    /// - `Existing`: `iblock` was already allocated; the range covers the
    ///   contiguous prefix of physical blocks starting from that position.
    /// - `NewlyAllocated`: `iblock` was a hole; new data blocks (and any
    ///   required indirect metadata blocks) were allocated and linked into
    ///   the tree. The range covers the freshly allocated data blocks.
    //
    // TODO: When a leaf block range is partially allocated, we walk the indirect tree
    // twice: once to discover the existing prefix, then again to find the hole
    // and allocate. A cursor over the leaf block could eliminate the second walk.
    pub(in crate::fs::fs_impls::ext2::inode) fn resolve_block_range(
        &mut self,
        fs: &Ext2,
        iblock: Iblock,
        max_blocks: u32,
    ) -> Result<ResolvedBlockRange> {
        if max_blocks == 0 {
            return_errno_with_message!(Errno::EINVAL, "zero block allocation requested");
        }

        let walk = self.walk_at(iblock)?;
        if walk.is_complete() {
            // The starting block already exists, so report its contiguous range.
            let existing_range = self.existing_contiguous_data_blocks(&walk, max_blocks)?;
            return Ok(ResolvedBlockRange::Existing(existing_range));
        }

        // Allocate the missing indirect metadata blocks, plus as many contiguous
        // data blocks as we can place before the current leaf boundary.
        let (indirect_blks, data_blks) = self.planned_allocation(&walk, max_blocks)?;
        let mut guard = self.allocate_blocks(fs, indirect_blks, data_blks, &walk)?;
        self.link_allocated_blocks(&guard, &walk)?;
        guard.commit();
        Ok(ResolvedBlockRange::NewlyAllocated(
            guard.data_blocks.clone(),
        ))
    }

    /// Truncates blocks to the new byte length (best-effort).
    ///
    /// This is a best-effort operation. Errors are logged but not propagated.
    /// Leaked blocks from partial failures are recoverable by e2fsck. Linux
    /// also follows this practice (see
    /// <https://elixir.bootlin.com/linux/v7.0/source/fs/ext2/inode.c#L1172>).
    pub(in crate::fs::fs_impls::ext2::inode) fn truncate_to_byte_len(
        &mut self,
        fs: &Ext2,
        new_size: usize,
    ) {
        // First logical block to free = ceil(new_size / block_size).
        let iblock = match Iblock::try_from(new_size.div_ceil(BLOCK_SIZE)) {
            Ok(ib) => ib,
            Err(_) => {
                error!("truncate: size exceeds ext2 limits, new_size={}", new_size);
                return;
            }
        };

        let walk = match self.walk_at(iblock) {
            Ok(w) => w,
            Err(err) => {
                error!("truncate: failed to compute block walk, err: {:?}", err);
                return;
            }
        };

        if walk.is_direct_data_block() {
            if let Err(err) = self.truncate_direct_slots(fs, walk.root_slot() as usize) {
                error!("truncate: truncate_direct_slots failed, err: {:?}", err);
            }
        } else if let Err(err) = self.truncate_indirect_path(fs, &walk) {
            error!("truncate: truncate_indirect_path failed, err: {:?}", err);
        }

        self.free_indirect_roots_after(fs, walk.root_slot() as usize);
    }

    /// Returns a conservative hole run starting from `iblock`, capped at
    /// `max_blocks`.
    ///
    /// The caller must have already verified that `iblock` is unallocated.
    /// Returns `0` if the block is actually allocated. The result may stop
    /// early at a direct/indirect region boundary, but it never overestimates:
    /// every block in the returned interval is known to be unmapped.
    pub(in crate::fs::fs_impls::ext2::inode) fn approx_hole_blocks(
        &self,
        iblock: Iblock,
        max_blocks: u32,
    ) -> Result<u32> {
        if max_blocks == 0 {
            return Ok(0);
        }
        let walk = self.walk_at(iblock)?;

        if walk.is_complete() {
            return Ok(0);
        }

        let leaf_level = walk.leaf_level();
        let hole_level = walk.hole_level();
        let subtree_indirect_levels = leaf_level - hole_level;

        let mut hole_len = 0u32;
        let ptrs = PTRS_PER_BLOCK as u32;

        // `hole_level` is where the first zero pointer was found. A leaf-level
        // hole needs a short slot scan; a higher-level zero pointer roots a
        // fully sparse subtree that can be skipped without probing each block.
        //
        // The computed length is intentionally conservative: it may stop before
        // the full hole ends, but it never crosses a possibly allocated pointer.
        if subtree_indirect_levels == 0 {
            let slot = walk.slot_at(hole_level);
            if hole_level == 0 {
                let scan_len = max_blocks.min(12 - slot);
                let scan_end = slot + scan_len;
                for &bid in &self.raw_block_ptrs.block_ptrs[slot as usize..scan_end as usize] {
                    if bid != 0 {
                        break;
                    }
                    hole_len += 1;
                }
            } else {
                let parent_bid = walk.parent_bid_at(hole_level);
                let mut indirect_block_manager = self.indirect_blocks_manager.lock();
                let indirect_block = indirect_block_manager.find(parent_bid)?;
                let scan_len = max_blocks.min(ptrs - slot);
                let scan_end = slot + scan_len;
                for idx in slot..scan_end {
                    let bid = indirect_block.read_bid(idx as usize)?;
                    if bid != 0 {
                        break;
                    }
                    hole_len += 1;
                }
            }
        } else {
            let subtree = ptrs.pow(subtree_indirect_levels as u32);
            let mut slot_offset = 0u32;
            for level in (hole_level + 1)..=leaf_level {
                slot_offset = slot_offset * ptrs + walk.slot_at(level);
            }
            hole_len = subtree - slot_offset;
        }

        Ok(hole_len.min(max_blocks))
    }

    fn truncate_direct_slots(&mut self, fs: &Ext2, start_idx: usize) -> Result<()> {
        let start = start_idx.min(12);
        for idx in start..12 {
            let bid = self.raw_block_ptrs.block_ptrs[idx];
            if bid == 0 {
                continue;
            }
            fs.free_blocks(bid, 1)?;
            self.raw_block_ptrs.block_ptrs[idx] = 0;
            self.raw_block_ptrs.sector_count = self
                .raw_block_ptrs
                .sector_count
                .saturating_sub(SECTORS_PER_BLOCK);
        }
        Ok(())
    }

    fn truncate_indirect_path(&mut self, fs: &Ext2, walk: &BlockPointerWalk) -> Result<()> {
        // If the truncation point is exactly at the start of an indirect
        // block (innermost slot = 0), trim trailing zero slots and re-walk the
        // shorter path. This raises the shared level so the whole block can be
        // detached at the parent without reading the lower subtree.
        let trimmed = self.walk_path(walk.trim_trailing_zero_slots())?;

        // `detach_level` is the highest level at which we detach and free a whole
        // subtree. Slots to the right of the cut below `detach_level` are cleared
        // by `free_indirect_right_side`.
        let detach_level = self.find_detach_level(walk, &trimmed)?;

        if let Some(detached_bid) = self.detach_subtree_root(walk, &trimmed, detach_level)? {
            self.free_block_subtree(fs, detached_bid, walk.subtree_indirect_levels(detach_level));
        }

        self.free_indirect_right_side(fs, walk, &trimmed, detach_level)?;

        Ok(())
    }

    fn find_detach_level(
        &self,
        walk: &BlockPointerWalk,
        trimmed: &BlockPointerWalk,
    ) -> Result<usize> {
        // Start from the deepest level if the walk was complete, or from the
        // level where the walk stopped on a zero pointer.
        let mut detach_level = if trimmed.is_complete() {
            trimmed.leaf_level()
        } else {
            trimmed.hole_level()
        };

        // Walk upward: if all entries to the left of the truncation slot in
        // an indirect block are zero, the entire block can be detached at the
        // parent level. Stop as soon as a non-zero kept entry is found.
        while detach_level > 0 {
            let current_bid = trimmed.parent_bid_at(detach_level);

            let mut indirect_blocks_manager = self.indirect_blocks_manager.lock();
            let block = indirect_blocks_manager.find(current_bid)?;
            let keep_entries = walk.slot_at(detach_level) as usize;
            let mut all_zero = true;
            for idx in 0..keep_entries {
                if block.read_bid(idx)? != 0 {
                    all_zero = false;
                    break;
                }
            }
            if !all_zero {
                break;
            }

            // Left side is all zeros; move up one level.
            detach_level -= 1;
        }

        Ok(detach_level)
    }

    fn detach_subtree_root(
        &mut self,
        walk: &BlockPointerWalk,
        trimmed: &BlockPointerWalk,
        detach_level: usize,
    ) -> Result<Option<Ext2Bid>> {
        let detached_bid = if detach_level == 0 {
            // Subtree root is referenced directly from `inode.block_ptrs[]`.
            let slot = walk.root_slot() as usize;
            let bid = self.raw_block_ptrs.block_ptrs[slot];
            self.raw_block_ptrs.block_ptrs[slot] = 0;
            bid
        } else {
            // Subtree root is a slot inside a parent indirect block.
            let parent_bid = trimmed.parent_bid_at(detach_level);
            let slot = walk.slot_at(detach_level) as usize;
            let mut indirect_blocks_manager = self.indirect_blocks_manager.lock();
            let parent_block = indirect_blocks_manager.find_mut(parent_bid)?;
            let bid = parent_block.read_bid(slot)?;
            parent_block.write_bid(slot, 0)?;
            bid
        };

        if detached_bid != 0 {
            Ok(Some(detached_bid))
        } else {
            Ok(None)
        }
    }

    fn free_indirect_right_side(
        &mut self,
        fs: &Ext2,
        walk: &BlockPointerWalk,
        trimmed: &BlockPointerWalk,
        detach_level: usize,
    ) -> Result<()> {
        let ptrs_per_block = PTRS_PER_BLOCK;
        // From `detach_level` down to level 1, clear all slots to the right of the
        // truncation slot and free their subtrees.
        for level in (1..=detach_level).rev() {
            let current_bid = trimmed.parent_bid_at(level);

            let start_idx = (walk.slot_at(level) as usize) + 1;
            let child_indirect_levels = walk.subtree_indirect_levels(level);
            // Collect block numbers before releasing the lock; `free_block_subtree`
            // may re-acquire the indirect block manager recursively.
            let child_blocks = {
                let mut indirect_blocks_manager = self.indirect_blocks_manager.lock();
                let block = indirect_blocks_manager.find_mut(current_bid)?;
                let mut child_blocks = Vec::new();
                for idx in start_idx..ptrs_per_block {
                    let bid = block.read_bid(idx)?;
                    if bid == 0 {
                        continue;
                    }
                    block.write_bid(idx, 0)?;
                    child_blocks.push(bid);
                }
                child_blocks
            };
            for bid in child_blocks {
                self.free_block_subtree(fs, bid, child_indirect_levels);
            }
        }
        Ok(())
    }

    fn free_indirect_roots_after(&mut self, fs: &Ext2, root_slot: usize) {
        // The conditions are cumulative: truncating before the single-indirect
        // root frees slots 12, 13, and 14; truncating at or before the
        // single-indirect root frees slots 13 and 14; truncating at or before
        // the double-indirect root frees slot 14.
        if root_slot < 12 {
            let bid = self.raw_block_ptrs.block_ptrs[12];
            if bid != 0 {
                self.raw_block_ptrs.block_ptrs[12] = 0;
                self.free_block_subtree(fs, bid, 1);
            }
        }
        if root_slot <= 12 {
            let bid = self.raw_block_ptrs.block_ptrs[13];
            if bid != 0 {
                self.raw_block_ptrs.block_ptrs[13] = 0;
                self.free_block_subtree(fs, bid, 2);
            }
        }
        if root_slot <= 13 {
            let bid = self.raw_block_ptrs.block_ptrs[14];
            if bid != 0 {
                self.raw_block_ptrs.block_ptrs[14] = 0;
                self.free_block_subtree(fs, bid, 3);
            }
        }
    }

    /// Translates a logical block number into a block-pointer path and walks
    /// the on-disk chain to resolve physical block addresses.
    ///
    /// After skipping the 12 direct blocks, the remaining block number is
    /// converted to base-1024 digits — each digit becomes a slot at one
    /// level of the indirect block tree.
    fn walk_at(&self, iblock: Iblock) -> Result<BlockPointerWalk> {
        let path = BlockPointerPath::new(iblock)?;
        self.walk_path(path)
    }

    /// Computes a `BlockPointerWalk` by walking the on-disk pointer chain.
    fn walk_path(&self, path: BlockPointerPath) -> Result<BlockPointerWalk> {
        let top_bid = self.raw_block_ptrs.block_ptrs[path.slots[0] as usize];
        let num_levels = path.num_levels;
        let mut visited_entries = SmallVec::new();
        if top_bid == 0 {
            // Zero pointer means the path is broken at level 0.
            return Ok(BlockPointerWalk {
                path,
                is_complete: false,
                visited_entries,
            });
        }
        visited_entries.push(top_bid);

        let mut indirect_blocks_manager = self.indirect_blocks_manager.lock();
        let mut parent_bid = top_bid;
        for level in 1..num_levels {
            let next_bid = indirect_blocks_manager
                .find(parent_bid)?
                .read_bid(path.slots[level] as usize)?;
            if next_bid == 0 {
                return Ok(BlockPointerWalk {
                    path,
                    is_complete: false,
                    visited_entries,
                });
            }
            visited_entries.push(next_bid);
            parent_bid = next_bid;
        }

        Ok(BlockPointerWalk {
            path,
            is_complete: true,
            visited_entries,
        })
    }

    fn existing_contiguous_data_blocks(
        &self,
        walk: &BlockPointerWalk,
        max_blocks: u32,
    ) -> Result<Range<Ext2Bid>> {
        if max_blocks == 0 {
            return_errno_with_message!(Errno::EINVAL, "zero block range requested");
        }
        // A partial walk means the logical block lands in a hole, so there is
        // no existing physical range to report.
        if !walk.is_complete() {
            return Ok(0..0);
        }
        // The last chain entry is the first existing data block for `iblock`.
        let first_bid = walk.bid_at(walk.leaf_level());
        if first_bid == 0 {
            return Ok(0..0);
        }

        let max_count = walk.max_blocks_in_leaf(max_blocks);
        let mut count = 1u32;
        let start_slot = walk.leaf_slot() as usize;

        if walk.is_direct_data_block() {
            // Direct blocks are stored inline in the inode, so scan forward in
            // `i_block[]` while the next physical block stays contiguous.
            while count < max_count {
                let slot = start_slot + count as usize;
                let Some(&next_bid) = self.raw_block_ptrs.block_ptrs.get(slot) else {
                    break;
                };
                if next_bid == 0 || next_bid != first_bid.saturating_add(count) {
                    break;
                }
                count += 1;
            }
        } else {
            // Indirect cases share the same contiguity rule, but the leaf slots
            // live in the final indirect block reached by the walk.
            let leaf_bid = walk.parent_bid_at(walk.leaf_level());
            let mut indirect_block_manager = self.indirect_blocks_manager.lock();
            let leaf_block = indirect_block_manager.find(leaf_bid)?;
            while count < max_count {
                let slot = start_slot + count as usize;
                let next_bid = leaf_block.read_bid(slot)?;
                if next_bid == 0 || next_bid != first_bid.saturating_add(count) {
                    break;
                }
                count += 1;
            }
        }

        Ok(first_bid..first_bid.saturating_add(count))
    }

    /// Returns how many indirect metadata blocks and data blocks are needed.
    ///
    /// The returned tuple is `(indirect_blks, data_blks)`. `indirect_blks`
    /// covers the missing metadata levels below the first hole in `walk`,
    /// while `data_blks` is the contiguous data range that can be placed in the
    /// current leaf without crossing its block-pointer boundary.
    fn planned_allocation(&self, walk: &BlockPointerWalk, max_blocks: u32) -> Result<(u32, u32)> {
        if max_blocks == 0 {
            return_errno_with_message!(Errno::EINVAL, "zero block allocation requested");
        }

        // Missing child levels correspond to indirect metadata blocks that
        // must be allocated before any data block can be linked into the tree.
        // Never allocate past the current leaf boundary even if the caller asks
        // for a larger contiguous range.
        let indirect_blks = walk.subtree_indirect_levels(walk.hole_level());
        let max_data_blks = walk.max_blocks_in_leaf(max_blocks);
        if indirect_blks > 0 {
            // If the path is incomplete, the fresh leaf starts empty, so we
            // can request the whole bounded data range immediately.
            return Ok((indirect_blks, max_data_blks));
        }

        // The indirect path already exists, so only count how many consecutive
        // empty slots remain in the current leaf.
        let start_slot = walk.leaf_slot() as usize;
        let mut count = 1u32;
        if walk.is_direct_data_block() {
            // Direct blocks live inline in the inode's `i_block[]`.
            while count < max_data_blks {
                let slot = start_slot + count as usize;
                let Some(&next_bid) = self.raw_block_ptrs.block_ptrs.get(slot) else {
                    break;
                };
                if next_bid != 0 {
                    break;
                }
                count += 1;
            }
        } else {
            // Indirect cases use the final indirect block as the allocation leaf.
            let leaf_bid = walk.parent_bid_at(walk.leaf_level());
            let mut indirect_blocks_manager = self.indirect_blocks_manager.lock();
            let leaf_block = indirect_blocks_manager.find(leaf_bid)?;
            while count < max_data_blks {
                let slot = start_slot + count as usize;
                if leaf_block.read_bid(slot)? != 0 {
                    break;
                }
                count += 1;
            }
        }

        Ok((0, count))
    }

    /// Frees all blocks in a pointer subtree.
    fn free_block_subtree(&mut self, fs: &Ext2, block_bid: Ext2Bid, indirect_levels: u32) {
        if block_bid == 0 {
            return;
        }

        if indirect_levels == 0 {
            if let Err(err) = fs.free_blocks(block_bid, 1) {
                // Best-effort free path logs errors and proceeds.
                error!(
                    "free_block_subtree: failed to free data block {}: {:?}",
                    block_bid, err
                );
                return;
            }
            self.raw_block_ptrs.sector_count = self
                .raw_block_ptrs
                .sector_count
                .saturating_sub(SECTORS_PER_BLOCK);
            return;
        }

        let child_blocks = {
            let mut indirect_blocks_manager = self.indirect_blocks_manager.lock();
            match indirect_blocks_manager.read_child_bids(block_bid) {
                Ok(children) => {
                    indirect_blocks_manager.remove(block_bid);
                    children
                }
                Err(_) => {
                    // Skip the damaged subtree after logging the read failure so
                    // cleanup can continue for the remaining subtree.
                    error!(
                        "free_block_subtree: failed to read indirect block {} (indirect_levels {})",
                        block_bid, indirect_levels
                    );
                    return;
                }
            }
        };

        for bid in child_blocks {
            self.free_block_subtree(fs, bid, indirect_levels - 1);
        }

        if let Err(err) = fs.free_blocks(block_bid, 1) {
            error!(
                "free_block_subtree: failed to free indirect block {}: {:?}",
                block_bid, err
            );
            return;
        }
        self.raw_block_ptrs.sector_count = self
            .raw_block_ptrs
            .sector_count
            .saturating_sub(SECTORS_PER_BLOCK);
    }

    /// Allocates indirect metadata blocks and data blocks for a missing path.
    ///
    /// Indirect blocks are allocated first so the eventual data range can be
    /// placed near the newly allocated metadata when possible. The returned
    /// guard frees all allocated blocks on drop unless `commit` is called after
    /// the blocks have been spliced into the tree.
    fn allocate_blocks<'a>(
        &self,
        fs: &'a Ext2,
        indirect_blks: u32,
        data_blks: u32,
        walk: &BlockPointerWalk,
    ) -> Result<BlockAllocGuard<'a>> {
        let mut alloc_goal = walk
            .visited_entries
            .last()
            .copied()
            .unwrap_or(self.raw_block_ptrs.block_ptrs[0]);

        // Allocate the missing indirect metadata first, then place data blocks
        // immediately after the metadata run when possible.
        let mut guard = BlockAllocGuard::new(fs, indirect_blks);

        let mut remaining_indirect_blks = indirect_blks;
        while remaining_indirect_blks > 0 {
            let allocated = fs.alloc_blocks(remaining_indirect_blks, alloc_goal)?;
            debug_assert!(allocated.end >= allocated.start);
            let allocated_count = allocated.end - allocated.start;
            debug_assert!(allocated_count > 0 && allocated_count <= remaining_indirect_blks);

            alloc_goal = allocated.end;
            remaining_indirect_blks -= allocated_count;
            guard.extend_indirect_blocks(allocated);
        }

        let data_blocks_range = fs.alloc_blocks(data_blks, alloc_goal)?;
        debug_assert!(data_blocks_range.end >= data_blocks_range.start);
        let allocated_count = data_blocks_range.end - data_blocks_range.start;
        guard.track_data_blocks(data_blocks_range.clone());
        debug_assert!(allocated_count > 0 && allocated_count <= data_blks);

        // We must wait until the blocks are initialized before we can proceed.
        // Otherwise, concurrent page faults may see uninitialized blocks.
        //
        // TODO: In the write path, if the entire block is going to be
        // overwritten, we should write the real payload instead of zeroing it.
        Self::zero_new_blocks(fs, &data_blocks_range)?;

        Ok(guard)
    }

    /// Zeroes newly allocated data blocks before exposing them via mapped reads.
    fn zero_new_blocks(fs: &Ext2, block_range: &Range<Ext2Bid>) -> Result<()> {
        let mut io_batch = IoBatch::with_capacity(1);

        let bio_segment = BioSegment::alloc(block_range.len(), BioDirection::ToDevice);
        let mut segment_writer = bio_segment.writer().unwrap();
        segment_writer.fill_zeros(block_range.len() * BLOCK_SIZE);
        fs.write_blocks_async(block_range.start, bio_segment, None, &mut io_batch)?;

        io_batch.wait_all()?;
        Ok(())
    }

    /// Splices allocated blocks into the block-pointer tree.
    ///
    /// If no indirect metadata blocks were allocated, this only fills data bids
    /// into existing leaf slots. Otherwise it builds the missing indirect chain,
    /// links the chain root into the first hole, and updates the inode sector
    /// count for both metadata and data blocks.
    fn link_allocated_blocks(
        &mut self,
        guard: &BlockAllocGuard,
        walk: &BlockPointerWalk,
    ) -> Result<()> {
        let indirect_blocks = &guard.indirect_blocks;
        let data_blocks = &guard.data_blocks;

        let total_blks = indirect_blocks.len() as u32 + data_blocks.len() as u32;
        let added_sectors = total_blks * SECTORS_PER_BLOCK;
        let new_block_count = self
            .raw_block_ptrs
            .sector_count
            .checked_add(added_sectors)
            .ok_or_else(|| Error::with_message(Errno::EIO, "inode block count overflow"))?;

        if indirect_blocks.is_empty() {
            // The path already exists; only fill data pointers into existing slots.
            self.fill_existing_leaf_slots(walk, data_blocks)?;
        } else {
            self.link_new_indirect_chain(walk, indirect_blocks, data_blocks)?;
        }

        self.raw_block_ptrs.sector_count = new_block_count;
        Ok(())
    }

    /// Fills data pointers into existing slots when no new indirect blocks are needed.
    fn fill_existing_leaf_slots(
        &mut self,
        walk: &BlockPointerWalk,
        data_blocks: &Range<Ext2Bid>,
    ) -> Result<()> {
        let hole_level = walk.hole_level();

        if hole_level == 0 {
            let slot = walk.root_slot() as usize;
            self.write_data_range_to_direct_slots(slot, data_blocks)?;
        } else {
            let parent_bid = walk.parent_bid_at(hole_level);
            let slot = walk.slot_at(hole_level) as usize;
            let mut indirect_blocks_manager = self.indirect_blocks_manager.lock();
            let parent_block = indirect_blocks_manager.find_mut(parent_bid)?;
            Self::write_data_range_to_indirect_block(parent_block, slot, data_blocks)?;
        }
        Ok(())
    }

    /// Builds a new indirect-block chain and links it into the tree.
    fn link_new_indirect_chain(
        &mut self,
        walk: &BlockPointerWalk,
        indirect_blocks: &[Ext2Bid],
        data_blocks: &Range<Ext2Bid>,
    ) -> Result<()> {
        let hole_level = walk.hole_level();
        let chain_root_bid = indirect_blocks[0];
        let mut indirect_blocks_manager = self.indirect_blocks_manager.lock();

        let result = (|| -> Result<()> {
            // Build the indirect blocks chain: each block points to the next;
            // the leaf block points to the allocated data range.
            debug_assert_eq!(
                indirect_blocks.len() as u32,
                walk.subtree_indirect_levels(hole_level)
            );
            let mut child_levels = walk.child_levels(hole_level);
            for (indirect_blk_idx, new_bid) in indirect_blocks.iter().copied().enumerate() {
                let level = child_levels
                    .next()
                    .expect("one child level per allocated indirect block");

                let mut block = IndirectBlock::alloc_new(new_bid)?;
                let start_slot = walk.slot_at(level) as usize;
                if indirect_blk_idx + 1 == indirect_blocks.len() {
                    Self::write_data_range_to_indirect_block(&mut block, start_slot, data_blocks)?;
                } else {
                    block.write_bid(start_slot, indirect_blocks[indirect_blk_idx + 1])?;
                }
                indirect_blocks_manager.insert(block)?;
            }

            // Splice the chain root into the parent pointer.
            if hole_level == 0 {
                let slot = walk.root_slot() as usize;
                if self.raw_block_ptrs.block_ptrs[slot] != 0 {
                    return_errno_with_message!(
                        Errno::EIO,
                        "block pointer changed during allocation"
                    );
                }
                self.raw_block_ptrs.block_ptrs[slot] = chain_root_bid;
            } else {
                let parent_bid = walk.parent_bid_at(hole_level);
                let parent_block = indirect_blocks_manager.find_mut(parent_bid)?;
                let slot = walk.slot_at(hole_level) as usize;
                if parent_block.read_bid(slot)? != 0 {
                    return_errno_with_message!(
                        Errno::EIO,
                        "block pointer changed during allocation"
                    );
                }
                parent_block.write_bid(slot, chain_root_bid)?;
            }

            Ok(())
        })();

        if let Err(err) = result {
            // Evict any indirect blocks already inserted into the cache.
            for &bid in indirect_blocks.iter() {
                indirect_blocks_manager.remove(bid);
            }
            return Err(err);
        }

        Ok(())
    }

    /// Writes a contiguous range of allocated data block IDs into direct
    /// `i_block[]` slots.
    ///
    /// Callers guarantee that the target slots are zero because `walk_path`
    /// confirmed the hole and the inode lock is held throughout.
    fn write_data_range_to_direct_slots(
        &mut self,
        start_slot: usize,
        data_range: &Range<Ext2Bid>,
    ) -> Result<()> {
        let end_slot = start_slot + data_range.len();
        let slots = &mut self.raw_block_ptrs.block_ptrs[start_slot..end_slot];

        for (entry, bid) in slots.iter_mut().zip(data_range.clone()) {
            debug_assert_eq!(*entry, 0);
            *entry = bid;
        }
        Ok(())
    }

    /// Writes a contiguous range of allocated data block IDs into an indirect
    /// block.
    ///
    /// The caller guarantees that every target slot is zero; the debug
    /// assertion keeps that invariant explicit during development.
    fn write_data_range_to_indirect_block(
        block: &mut IndirectBlock,
        start_slot: usize,
        data_range: &Range<Ext2Bid>,
    ) -> Result<()> {
        let end_slot = start_slot + data_range.len();

        for (slot, bid) in (start_slot..end_slot).zip(data_range.clone()) {
            debug_assert_eq!(block.read_bid(slot).unwrap_or(0), 0);
            block.write_bid(slot, bid)?;
        }
        Ok(())
    }
}

/// On-disk block pointer state mirrored from `RawInode`.
///
/// The mirrored fields are `i_blocks` (sector count) and the 15-entry
/// `i_block[]` array.
///
/// This copy exists to break a locking cycle between `Inode` and
/// `PageCacheBackend`. Both need to look up or allocate blocks, but `Inode`
/// also drives `PageCacheBackend` operations. Giving both sides access to this
/// struct instead of routing everything through the `InodeInner` lock avoids a
/// deadlock. The struct is the authoritative in-memory state and is written
/// back to the inode on sync.
#[derive(Clone, Copy, Debug)]
pub(in crate::fs::fs_impls::ext2::inode) struct RawBlockPtrs {
    pub sector_count: u32,
    pub block_ptrs: [u32; RAW_BLOCK_PTRS_LEN],
}

impl RawBlockPtrs {
    /// Creates a `RawBlockPtrs` from the given sector count and pointer array.
    pub(in crate::fs::fs_impls::ext2::inode) fn new(
        sector_count: u32,
        block_ptrs: [u32; RAW_BLOCK_PTRS_LEN],
    ) -> Self {
        Self {
            sector_count,
            block_ptrs,
        }
    }

    /// Reads the ext2 special-file device encoding stored in `i_block`.
    pub(in crate::fs::fs_impls::ext2::inode) fn read_device_id(&self) -> u64 {
        let (major, minor) = if self.block_ptrs[0] != 0 {
            let old_encoded_device = self.block_ptrs[0];
            // Old_decode_dev: (major << 8) | minor with 8-bit major/minor.
            (
                ((old_encoded_device >> 8) & 0xFF),
                (old_encoded_device & 0xFF),
            )
        } else {
            let encoded_device = self.block_ptrs[1];
            // Decode the extended major/minor bit layout.
            (
                ((encoded_device & 0xFFF00) >> 8),
                ((encoded_device & 0xFF) | ((encoded_device >> 12) & 0xFFF00)),
            )
        };

        encode_device_numbers(major, minor)
    }

    /// Writes a device ID into the ext2 special-file `i_block` layout.
    pub(in crate::fs::fs_impls::ext2::inode) fn write_device_id(&mut self, device_id: u64) {
        let (major, minor) = decode_device_numbers(device_id);

        // Old_valid_dev: MAJOR/MINOR must both fit in 8 bits.
        if major < 256 && minor < 256 {
            self.block_ptrs[0] = (major << 8) | minor;
            self.block_ptrs[1] = 0;
        } else {
            self.block_ptrs[0] = 0;
            self.block_ptrs[1] = (minor & 0xFF) | (major << 8) | ((minor & !0xFF) << 12);
            self.block_ptrs[2] = 0;
        }
    }
}

/// Result of resolving a physical block range from a logical block position.
#[derive(Debug)]
pub(in crate::fs::fs_impls::ext2::inode) enum ResolvedBlockRange {
    /// The physical range already exists on device.
    Existing(Range<Ext2Bid>),
    /// The physical range was freshly allocated on device.
    NewlyAllocated(Range<Ext2Bid>),
}

/// Full block-pointer path from `i_block[]` to a logical data block.
///
/// `slots[0]` is always the slot in the inode's `i_block[]` array.
/// `slots[1..num_levels]` are the successive slots inside indirect
/// blocks, from outermost to innermost.
#[derive(Clone, Copy, Debug)]
struct BlockPointerPath {
    slots: [u32; MAX_BLOCK_POINTER_LEVELS],
    num_levels: usize,
}

impl BlockPointerPath {
    /// Creates a block-pointer path for a logical block index.
    fn new(iblock: Iblock) -> Result<Self> {
        let ptrs = PTRS_PER_BLOCK as u32;
        let ptr_shift = ptrs.trailing_zeros();
        let ptr_mask = ptrs - 1;
        let double_blocks = 1u32 << (ptr_shift * 2);

        if iblock < 12 {
            return Ok(Self::from_slots([iblock, 0, 0, 0], 1));
        }
        let block = iblock - 12;

        if block < ptrs {
            return Ok(Self::from_slots([12, block, 0, 0], 2));
        }
        let block = block - ptrs;

        if block < double_blocks {
            return Ok(Self::from_slots(
                [13, block >> ptr_shift, block & ptr_mask, 0],
                3,
            ));
        }
        let block = block - double_blocks;

        if (block >> (ptr_shift * 2)) < ptrs {
            return Ok(Self::from_slots(
                [
                    14,
                    block >> (ptr_shift * 2),
                    (block >> ptr_shift) & ptr_mask,
                    block & ptr_mask,
                ],
                4,
            ));
        }

        return_errno_with_message!(Errno::EINVAL, "block number exceeds ext2 pointer tree");
    }

    fn from_slots(slots: [u32; MAX_BLOCK_POINTER_LEVELS], num_levels: usize) -> Self {
        debug_assert!((1..=MAX_BLOCK_POINTER_LEVELS).contains(&num_levels));
        Self { slots, num_levels }
    }
}

/// Traversal state from an inode's `i_block[]` array to a logical data block.
///
/// The state combines the slot path selected by the logical block index with
/// the block IDs observed in the on-disk pointer chain.
///
/// For a 4 KiB block size (1024 pointers per indirect block):
///
/// ```text
///   Logical block 0..11 (direct):
///     levels=1:  i_block[0..12]  →  data block
///
///   Logical block 12..1035 (single indirect):
///     levels=2:  i_block[12]  →  indirect[slot]  →  data block
///
///   Logical block 1036..1049611 (double indirect):
///     levels=3:  i_block[13]  →  L1[a]  →  L2[b]  →  data block
///
///   Logical block 1049612..1074791435 (triple indirect):
///     levels=4:  i_block[14]  →  L1[a]  →  L2[b]  →  L3[c]  →  data block
/// ```
///
/// `level` is a 0-based index into this chain: level 0 is always the inode
/// `i_block[]` slot, and levels 1.. are successive indirect-block slots.
#[derive(Debug)]
struct BlockPointerWalk {
    /// Slot path from inode `i_block[]` to the target logical block.
    ///
    /// This is pure addressing metadata; physical block numbers read while
    /// following the path are recorded in `visited_entries`.
    path: BlockPointerPath,
    /// Whether every pointer in the path was non-zero.
    is_complete: bool,
    /// Non-zero entries visited so far, one per completed level.
    ///
    /// If the walk is incomplete, the first hole is at
    /// `visited_entries.len()`.
    visited_entries: SmallVec<[Ext2Bid; MAX_BLOCK_POINTER_LEVELS]>,
}

impl BlockPointerWalk {
    /// Returns the root slot in inode `i_block[]`.
    fn root_slot(&self) -> u32 {
        self.path.slots[0]
    }

    /// Returns whether this walk names an inline direct data-block slot.
    fn is_direct_data_block(&self) -> bool {
        self.root_slot() < 12
    }

    /// Returns the total number of levels in the pointer chain.
    #[cfg(ktest)]
    fn num_levels(&self) -> usize {
        self.path.num_levels
    }

    /// Returns the slot at the given level of the block pointer tree.
    fn slot_at(&self, level: usize) -> u32 {
        debug_assert!(level < self.path.num_levels);
        self.path.slots[level]
    }

    /// Returns the slot at the deepest (leaf) level.
    fn leaf_slot(&self) -> u32 {
        self.slot_at(self.leaf_level())
    }

    /// Returns the index (0-based) of the deepest (leaf) level in the pointer chain.
    fn leaf_level(&self) -> usize {
        self.path.num_levels - 1
    }

    /// Trims trailing zero slots from the innermost levels.
    ///
    /// Used by `truncate_to_byte_len` to raise the shared path level when the
    /// truncation point sits at the start of an indirect block.
    fn trim_trailing_zero_slots(&self) -> BlockPointerPath {
        let mut num_levels = self.path.num_levels;
        while num_levels > 1 && self.path.slots[num_levels - 1] == 0 {
            num_levels -= 1;
        }

        BlockPointerPath::from_slots(self.path.slots, num_levels)
    }

    /// Returns the indirect-block levels in the subtree rooted at `root_level`.
    ///
    /// Equals `leaf_level() - root_level`: the number of indirect blocks that
    /// must still be traversed from the node at `root_level` down to data
    /// blocks.
    /// Pass this directly as the `indirect_levels` argument to `free_block_subtree`.
    fn subtree_indirect_levels(&self, root_level: usize) -> u32 {
        debug_assert!(root_level <= self.leaf_level());
        (self.leaf_level() - root_level) as u32
    }

    /// Returns the child levels below `level`, from outermost to innermost.
    fn child_levels(&self, level: usize) -> impl Iterator<Item = usize> {
        debug_assert!(level <= self.leaf_level());
        (level + 1)..=self.leaf_level()
    }

    /// Returns the maximum block count before crossing this walk's leaf boundary.
    fn max_blocks_in_leaf(&self, max_blocks: u32) -> u32 {
        let boundary = if self.is_direct_data_block() {
            11 - self.leaf_slot()
        } else {
            PTRS_PER_BLOCK as u32 - 1 - self.leaf_slot()
        };
        max_blocks.min(boundary + 1)
    }

    fn is_complete(&self) -> bool {
        self.is_complete
    }

    fn bid_at(&self, level: usize) -> Ext2Bid {
        debug_assert!(level < self.visited_entries.len());
        self.visited_entries[level]
    }

    fn parent_bid_at(&self, level: usize) -> Ext2Bid {
        debug_assert!(level > 0);
        self.bid_at(level - 1)
    }

    fn hole_level(&self) -> usize {
        debug_assert!(!self.is_complete());
        self.visited_entries.len()
    }
}

/// Rollback guard for metadata and data blocks allocated through the block-pointer tree.
#[derive(Debug)]
struct BlockAllocGuard<'a> {
    fs: &'a Ext2,
    indirect_blocks: Vec<Ext2Bid>,
    data_blocks: Range<Ext2Bid>,
    committed: bool,
}

impl<'a> BlockAllocGuard<'a> {
    fn new(fs: &'a Ext2, indirect_blocks: u32) -> Self {
        Self {
            fs,
            indirect_blocks: Vec::with_capacity(indirect_blocks as usize),
            data_blocks: Range { start: 0, end: 0 },
            committed: false,
        }
    }

    fn extend_indirect_blocks(&mut self, indirect_blocks: Range<Ext2Bid>) {
        self.indirect_blocks.extend(indirect_blocks);
    }

    fn track_data_blocks(&mut self, data_blocks: Range<Ext2Bid>) {
        debug_assert!(self.data_blocks.is_empty());
        self.data_blocks = data_blocks;
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for BlockAllocGuard<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }

        for &bid in self.indirect_blocks.iter() {
            if let Err(err) = self.fs.free_blocks(bid, 1) {
                error!(
                    "failed to free indirect block {} in rollback: {:?}",
                    bid, err
                );
            }
        }

        if self.data_blocks.is_empty() {
            return;
        }
        let free_result = self
            .fs
            .free_blocks(self.data_blocks.start, self.data_blocks.len() as u32);
        if let Err(err) = free_result {
            error!("failed to free data blocks in rollback: {:?}", err);
        }
    }
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::ktest;

    use super::*;
    use crate::{
        fs::fs_impls::ext2::test_utils::{Ext2FixtureBuilder, write_indirect_ptr},
        prelude::*,
        time::clocks,
    };

    fn alloc_single_block(tree: &mut BlockPtrTree, fs: &Ext2, iblock: Iblock) -> Result<Ext2Bid> {
        let step = tree.resolve_block_range(fs, iblock, 1)?;
        let range = match step {
            ResolvedBlockRange::Existing(r) | ResolvedBlockRange::NewlyAllocated(r) => r,
        };
        assert!(!range.is_empty());
        Ok(range.start)
    }

    fn expect_allocated(step: ResolvedBlockRange) -> Range<Ext2Bid> {
        match step {
            ResolvedBlockRange::NewlyAllocated(range) => range,
            ResolvedBlockRange::Existing(range) => {
                panic!("expected allocated range, got existing range {:?}", range)
            }
        }
    }

    fn expect_existing(step: ResolvedBlockRange) -> Range<Ext2Bid> {
        match step {
            ResolvedBlockRange::Existing(range) => range,
            ResolvedBlockRange::NewlyAllocated(range) => {
                panic!(
                    "expected existing range, got newly allocated range {:?}",
                    range
                )
            }
        }
    }

    fn make_block_ptr_tree(
        block_ptrs: [u32; RAW_BLOCK_PTRS_LEN],
        sector_count: u32,
        fs: &Arc<Ext2>,
    ) -> BlockPtrTree {
        BlockPtrTree::new(
            RawBlockPtrs::new(sector_count, block_ptrs),
            Arc::downgrade(fs),
        )
    }

    #[ktest]
    fn block_ptr_tree_direct_and_indirect_ok() {
        let f = Ext2FixtureBuilder::new(2, 256).build().unwrap();
        let disk = &f.disk;

        let ptrs = PTRS_PER_BLOCK as u32;
        let ptrs_bits = ptrs.trailing_zeros();
        let double_blocks = 1u32 << (ptrs_bits * 2);

        let indirect_bid = 40u32;
        let indirect_index = 5u32;
        let existing_bid = 77u32;
        write_indirect_ptr(disk.as_ref(), indirect_bid, indirect_index, existing_bid);

        let double_l1_bid = 41u32;
        let double_l2_bid = 42u32;
        let double_data_bid = 78u32;
        write_indirect_ptr(disk.as_ref(), double_l1_bid, 3, double_l2_bid);
        write_indirect_ptr(disk.as_ref(), double_l2_bid, 4, double_data_bid);

        let triple_l1_bid = 43u32;
        let triple_l2_bid = 44u32;
        let triple_l3_bid = 45u32;
        let triple_data_bid = 79u32;
        write_indirect_ptr(disk.as_ref(), triple_l1_bid, 2, triple_l2_bid);
        write_indirect_ptr(disk.as_ref(), triple_l2_bid, 3, triple_l3_bid);
        write_indirect_ptr(disk.as_ref(), triple_l3_bid, 4, triple_data_bid);

        let mut block_ptrs = [0u32; RAW_BLOCK_PTRS_LEN];
        block_ptrs[0] = 11;
        block_ptrs[12] = indirect_bid;
        block_ptrs[13] = double_l1_bid;
        block_ptrs[14] = triple_l1_bid;
        let block_ptr_tree = make_block_ptr_tree(block_ptrs, 0, &f.ext2);

        // Cover exact transition boundaries across all block-pointer tree levels.
        let direct_walk = block_ptr_tree.walk_at(0).unwrap();
        assert_eq!(direct_walk.num_levels(), 1);
        assert_eq!(direct_walk.root_slot(), 0);
        assert_eq!(direct_walk.max_blocks_in_leaf(u32::MAX), 12);

        let direct_last_walk = block_ptr_tree.walk_at(11).unwrap();
        assert_eq!(direct_last_walk.num_levels(), 1);
        assert_eq!(direct_last_walk.root_slot(), 11);
        assert_eq!(direct_last_walk.max_blocks_in_leaf(u32::MAX), 1);

        let indirect_first_walk = block_ptr_tree.walk_at(12).unwrap();
        assert_eq!(indirect_first_walk.num_levels(), 2);
        assert_eq!(indirect_first_walk.root_slot(), 12);
        assert_eq!(indirect_first_walk.slot_at(1), 0);

        let indirect_walk = block_ptr_tree.walk_at(12 + indirect_index).unwrap();
        assert_eq!(indirect_walk.num_levels(), 2);
        assert_eq!(indirect_walk.root_slot(), 12);
        assert_eq!(indirect_walk.slot_at(1), indirect_index);

        let indirect_last_iblock = 12 + ptrs - 1;
        let indirect_last_walk = block_ptr_tree.walk_at(indirect_last_iblock).unwrap();
        assert_eq!(indirect_last_walk.num_levels(), 2);
        assert_eq!(indirect_last_walk.root_slot(), 12);
        assert_eq!(indirect_last_walk.slot_at(1), ptrs - 1);
        assert_eq!(indirect_last_walk.max_blocks_in_leaf(u32::MAX), 1);

        let first_double_iblock = 12 + ptrs;
        let first_double_walk = block_ptr_tree.walk_at(first_double_iblock).unwrap();
        assert_eq!(first_double_walk.num_levels(), 3);
        assert_eq!(first_double_walk.root_slot(), 13);
        assert_eq!(first_double_walk.slot_at(1), 0);
        assert_eq!(first_double_walk.slot_at(2), 0);

        let double_iblock = 12 + ptrs + (3 << ptrs_bits) + 4;
        let double_walk = block_ptr_tree.walk_at(double_iblock).unwrap();
        assert_eq!(double_walk.num_levels(), 3);
        assert_eq!(double_walk.root_slot(), 13);
        assert_eq!(double_walk.slot_at(1), 3);
        assert_eq!(double_walk.slot_at(2), 4);

        let first_triple_iblock = 12 + ptrs + double_blocks;
        let first_triple_walk = block_ptr_tree.walk_at(first_triple_iblock).unwrap();
        assert_eq!(first_triple_walk.num_levels(), 4);
        assert_eq!(first_triple_walk.root_slot(), 14);
        assert_eq!(first_triple_walk.slot_at(1), 0);
        assert_eq!(first_triple_walk.slot_at(2), 0);
        assert_eq!(first_triple_walk.slot_at(3), 0);

        let triple_iblock =
            12 + ptrs + double_blocks + (2 << (ptrs_bits * 2)) + (3 << ptrs_bits) + 4;
        let triple_walk = block_ptr_tree.walk_at(triple_iblock).unwrap();
        assert_eq!(triple_walk.num_levels(), 4);
        assert_eq!(triple_walk.root_slot(), 14);
        assert_eq!(triple_walk.slot_at(1), 2);
        assert_eq!(triple_walk.slot_at(2), 3);
        assert_eq!(triple_walk.slot_at(3), 4);

        // Verify block lookup resolves direct/indirect/double/triple chains.
        assert_eq!(block_ptr_tree.lookup_block(0).unwrap(), Some(11));
        assert_eq!(block_ptr_tree.lookup_block(1).unwrap(), None);
        assert_eq!(
            block_ptr_tree.lookup_block(12 + indirect_index).unwrap(),
            Some(existing_bid)
        );
        assert_eq!(
            block_ptr_tree.lookup_block(double_iblock).unwrap(),
            Some(double_data_bid)
        );
        assert_eq!(
            block_ptr_tree.lookup_block(triple_iblock).unwrap(),
            Some(triple_data_bid)
        );
    }

    #[ktest]
    fn block_ptr_tree_get_block_range_returns_contiguous_runs() {
        let f = Ext2FixtureBuilder::new(2, 256).build().unwrap();
        let disk = &f.disk;

        let indirect_bid = 40u32;
        write_indirect_ptr(disk.as_ref(), indirect_bid, 0, 70);
        write_indirect_ptr(disk.as_ref(), indirect_bid, 1, 71);
        write_indirect_ptr(disk.as_ref(), indirect_bid, 2, 72);
        write_indirect_ptr(disk.as_ref(), indirect_bid, 3, 90);

        let mut block_ptrs = [0u32; RAW_BLOCK_PTRS_LEN];
        block_ptrs[0] = 11;
        block_ptrs[1] = 12;
        block_ptrs[2] = 13;
        block_ptrs[12] = indirect_bid;
        let block_ptr_tree = make_block_ptr_tree(block_ptrs, 0, &f.ext2);

        assert_eq!(block_ptr_tree.lookup_block_range(0, 4).unwrap(), 11..14);
        assert_eq!(block_ptr_tree.lookup_block(0).unwrap(), Some(11));
        assert_eq!(block_ptr_tree.lookup_block_range(12, 4).unwrap(), 70..73);
        assert_eq!(block_ptr_tree.lookup_block_range(15, 4).unwrap(), 90..91);
        assert!(block_ptr_tree.lookup_block_range(3, 4).unwrap().is_empty());
    }

    #[ktest]
    fn block_ptr_tree_out_of_range_iblock_returns_err() {
        let f = Ext2FixtureBuilder::new(2, 256).build().unwrap();
        let disk = &f.disk;

        let ptrs = PTRS_PER_BLOCK as u64;
        let direct = 12u64;
        let indirect = ptrs;
        let double_blocks = 1u64 << (ptrs.trailing_zeros() * 2);
        let triple_blocks = 1u64 << (ptrs.trailing_zeros() * 3);
        let max_iblock = direct + indirect + double_blocks + triple_blocks - 1;
        let too_big = (max_iblock + 1) as u32;

        let block_ptr_tree = make_block_ptr_tree([0; RAW_BLOCK_PTRS_LEN], 0, &f.ext2);

        // Accept the maximum valid logical block and reject the next one.
        block_ptr_tree.walk_at(max_iblock as u32).unwrap();

        let too_big_err = block_ptr_tree.walk_at(too_big).unwrap_err();
        assert_eq!(too_big_err.error(), Errno::EINVAL);

        let get_too_big_err = block_ptr_tree.lookup_block(too_big).unwrap_err();
        assert_eq!(get_too_big_err.error(), Errno::EINVAL);

        // Any zero pointer on the path is treated as a hole (None).
        let mut ptrs_for_indirect_hole = [0u32; RAW_BLOCK_PTRS_LEN];
        ptrs_for_indirect_hole[12] = 40;
        let indirect_hole_block_ptr_tree = make_block_ptr_tree(ptrs_for_indirect_hole, 0, &f.ext2);
        assert_eq!(
            indirect_hole_block_ptr_tree.lookup_block(12 + 7).unwrap(),
            None
        );

        let mut ptrs_for_double_hole = [0u32; RAW_BLOCK_PTRS_LEN];
        ptrs_for_double_hole[13] = 41;
        write_indirect_ptr(disk.as_ref(), 41, 3, 0);
        let double_hole_block_ptr_tree = make_block_ptr_tree(ptrs_for_double_hole, 0, &f.ext2);
        let double_hole_iblock = 12 + (ptrs as u32) + (3 << ptrs.trailing_zeros()) + 4;
        assert_eq!(
            double_hole_block_ptr_tree
                .lookup_block(double_hole_iblock)
                .unwrap(),
            None
        );

        let mut ptrs_for_triple_hole = [0u32; RAW_BLOCK_PTRS_LEN];
        ptrs_for_triple_hole[14] = 43;
        write_indirect_ptr(disk.as_ref(), 43, 2, 44);
        write_indirect_ptr(disk.as_ref(), 44, 3, 0);
        let triple_hole_block_ptr_tree = make_block_ptr_tree(ptrs_for_triple_hole, 0, &f.ext2);
        let triple_hole_iblock = 12
            + (ptrs as u32)
            + (double_blocks as u32)
            + (2 << (ptrs.trailing_zeros() * 2))
            + (3 << ptrs.trailing_zeros())
            + 4;
        assert_eq!(
            triple_hole_block_ptr_tree
                .lookup_block(triple_hole_iblock)
                .unwrap(),
            None
        );
    }

    #[ktest]
    fn approx_hole_blocks_scans() {
        clocks::init_for_ktest();
        let f = Ext2FixtureBuilder::new(2, 256).build().unwrap();
        let disk = &f.disk;
        let ptrs = PTRS_PER_BLOCK;
        let ptrs_bits = (ptrs as u32).trailing_zeros();

        // Direct blocks scan only the inode's direct pointer array and stop
        // before the first allocated slot.
        let mut direct_ptrs = [0u32; RAW_BLOCK_PTRS_LEN];
        direct_ptrs[2] = 91;
        let direct_block_ptr_tree = make_block_ptr_tree(direct_ptrs, 0, &f.ext2);
        assert_eq!(direct_block_ptr_tree.approx_hole_blocks(0, 1).unwrap(), 1);
        assert_eq!(direct_block_ptr_tree.approx_hole_blocks(0, 8).unwrap(), 2);
        assert_eq!(direct_block_ptr_tree.approx_hole_blocks(1, 8).unwrap(), 1);
        assert_eq!(direct_block_ptr_tree.approx_hole_blocks(2, 8).unwrap(), 0);

        // A leaf-level hole inside a present indirect block is scanned slot by
        // slot, just like the direct case.
        let indirect_bid = 40u32;
        write_indirect_ptr(disk.as_ref(), indirect_bid, 2, 77);
        let mut indirect_ptrs = [0u32; RAW_BLOCK_PTRS_LEN];
        indirect_ptrs[12] = indirect_bid;
        let indirect_block_ptr_tree = make_block_ptr_tree(indirect_ptrs, 0, &f.ext2);
        assert_eq!(
            indirect_block_ptr_tree.approx_hole_blocks(12, 1).unwrap(),
            1
        );
        assert_eq!(
            indirect_block_ptr_tree.approx_hole_blocks(12, 8).unwrap(),
            2
        );
        assert_eq!(
            indirect_block_ptr_tree.approx_hole_blocks(13, 8).unwrap(),
            1
        );
        assert_eq!(
            indirect_block_ptr_tree.approx_hole_blocks(14, 8).unwrap(),
            0
        );

        // A missing double-indirect root means the whole remaining subtree is
        // sparse, so the helper can skip it without reading child blocks.
        let double_hole_iblock = 12 + (ptrs as u32) + (3 << ptrs_bits) + 4;
        let expected_double_hole = ptrs.pow(2) as u32 - ((3 * ptrs) as u32 + 4);
        let missing_double_root_map = make_block_ptr_tree([0u32; RAW_BLOCK_PTRS_LEN], 0, &f.ext2);
        assert_eq!(
            missing_double_root_map
                .approx_hole_blocks(double_hole_iblock, expected_double_hole + 1)
                .unwrap(),
            expected_double_hole
        );

        // If the double-indirect root exists but the selected child pointer is
        // zero, only that child subtree is known to be sparse.
        let double_l1_bid = 41u32;
        write_indirect_ptr(disk.as_ref(), double_l1_bid, 3, 0);
        let mut double_ptrs = [0u32; RAW_BLOCK_PTRS_LEN];
        double_ptrs[13] = double_l1_bid;
        let double_middle_hole_map = make_block_ptr_tree(double_ptrs, 0, &f.ext2);
        assert_eq!(
            double_middle_hole_map
                .approx_hole_blocks(double_hole_iblock, ptrs as u32)
                .unwrap(),
            ptrs as u32 - 4
        );

        // Triple-indirect roots follow the same rule: a missing top-level root
        // covers the rest of the selected triple-indirect region.
        let triple_hole_iblock =
            12 + ptrs as u32 + ptrs.pow(2) as u32 + (2 << (ptrs_bits * 2)) + (3 << ptrs_bits) + 4;
        let expected_triple_top_hole =
            ptrs.pow(3) as u32 - ((2 * ptrs.pow(2)) as u32 + (3 * ptrs) as u32 + 4);
        let missing_triple_root_map = make_block_ptr_tree([0u32; RAW_BLOCK_PTRS_LEN], 0, &f.ext2);
        assert_eq!(
            missing_triple_root_map
                .approx_hole_blocks(triple_hole_iblock, expected_triple_top_hole + 1)
                .unwrap(),
            expected_triple_top_hole
        );

        // With the triple root present, a zero first-level child bounds the
        // sparse run to that child subtree.
        let triple_l1_bid = 42u32;
        write_indirect_ptr(disk.as_ref(), triple_l1_bid, 2, 0);
        let mut triple_l1_ptrs = [0u32; RAW_BLOCK_PTRS_LEN];
        triple_l1_ptrs[14] = triple_l1_bid;
        let triple_l1_hole_map = make_block_ptr_tree(triple_l1_ptrs, 0, &f.ext2);
        let expected_triple_l1_hole = ptrs.pow(2) as u32 - ((3 * ptrs) as u32 + 4);
        assert_eq!(
            triple_l1_hole_map
                .approx_hole_blocks(triple_hole_iblock, expected_triple_l1_hole + 1)
                .unwrap(),
            expected_triple_l1_hole
        );

        // Once the first-level child exists, a zero second-level child narrows
        // the hole to the remaining slots in that leaf subtree.
        let triple_l2_bid = 43u32;
        write_indirect_ptr(disk.as_ref(), triple_l1_bid, 2, triple_l2_bid);
        write_indirect_ptr(disk.as_ref(), triple_l2_bid, 3, 0);
        let triple_l2_hole_map = make_block_ptr_tree(triple_l1_ptrs, 0, &f.ext2);
        assert_eq!(
            triple_l2_hole_map
                .approx_hole_blocks(triple_hole_iblock, ptrs as u32)
                .unwrap(),
            ptrs as u32 - 4
        );
    }

    #[ktest]
    fn block_alloc_direct_range_ok() {
        let f = Ext2FixtureBuilder::new(1, 256)
            .with_free_blocks(64, 64)
            .build()
            .unwrap();
        let ext2 = &f.ext2;

        let mut block_ptr_tree = make_block_ptr_tree([0u32; RAW_BLOCK_PTRS_LEN], 0, &f.ext2);
        let free_before = ext2.super_block().free_blocks_count();
        let allocated_range =
            expect_allocated(block_ptr_tree.resolve_block_range(ext2, 0, 3).unwrap());
        let free_after = ext2.super_block().free_blocks_count();

        assert_eq!(allocated_range.end - allocated_range.start, 3);
        assert_eq!(
            block_ptr_tree.raw_block_ptrs.block_ptrs[0],
            allocated_range.start
        );
        assert_eq!(
            block_ptr_tree.raw_block_ptrs.block_ptrs[1],
            allocated_range.start + 1
        );
        assert_eq!(
            block_ptr_tree.raw_block_ptrs.block_ptrs[2],
            allocated_range.start + 2
        );
        assert_eq!(
            block_ptr_tree.lookup_block_range(0, 3).unwrap(),
            allocated_range.clone()
        );
        assert_eq!(
            expect_existing(block_ptr_tree.resolve_block_range(ext2, 0, 1).unwrap()).start,
            allocated_range.start
        );
        assert_eq!(
            block_ptr_tree.raw_block_ptrs.sector_count,
            SECTORS_PER_BLOCK * 3
        );
        assert_eq!(free_before - free_after, 3);
    }

    #[ktest]
    fn block_alloc_indirect_range_ok() {
        let f = Ext2FixtureBuilder::new(1, 256)
            .with_free_blocks(64, 64)
            .build()
            .unwrap();
        let ext2 = &f.ext2;

        let mut block_ptr_tree = make_block_ptr_tree([0u32; RAW_BLOCK_PTRS_LEN], 0, &f.ext2);
        let free_before = ext2.super_block().free_blocks_count();
        let allocated_range =
            expect_allocated(block_ptr_tree.resolve_block_range(ext2, 12, 4).unwrap());
        let free_after = ext2.super_block().free_blocks_count();

        assert_ne!(block_ptr_tree.raw_block_ptrs.block_ptrs[12], 0);
        assert_eq!(allocated_range.end - allocated_range.start, 4);
        assert_eq!(
            block_ptr_tree.lookup_block_range(12, 4).unwrap(),
            allocated_range.clone()
        );
        assert_eq!(
            block_ptr_tree.lookup_block(12).unwrap(),
            Some(allocated_range.start)
        );
        assert_eq!(
            block_ptr_tree.raw_block_ptrs.sector_count,
            SECTORS_PER_BLOCK * 5
        );
        assert_eq!(free_before - free_after, 5);
    }

    #[ktest]
    fn truncate_indirect_frees_shared_path() {
        let f = Ext2FixtureBuilder::new(1, 256)
            .with_free_blocks(64, 64)
            .build()
            .unwrap();
        let ext2 = &f.ext2;
        let ptrs = PTRS_PER_BLOCK as u32;
        let first_double_iblock = 12 + ptrs;

        let mut block_ptr_tree = make_block_ptr_tree([0u32; RAW_BLOCK_PTRS_LEN], 0, &f.ext2);
        alloc_single_block(&mut block_ptr_tree, ext2, first_double_iblock).unwrap();
        alloc_single_block(&mut block_ptr_tree, ext2, first_double_iblock + 1).unwrap();
        alloc_single_block(&mut block_ptr_tree, ext2, first_double_iblock + 2).unwrap();

        block_ptr_tree.truncate_to_byte_len(ext2, (first_double_iblock as usize + 1) * BLOCK_SIZE);
        assert!(
            block_ptr_tree
                .lookup_block(first_double_iblock)
                .unwrap()
                .is_some()
        );
        assert_eq!(
            block_ptr_tree
                .lookup_block(first_double_iblock + 1)
                .unwrap(),
            None
        );
        assert_eq!(
            block_ptr_tree
                .lookup_block(first_double_iblock + 2)
                .unwrap(),
            None
        );
    }

    #[ktest]
    fn truncate_releases_all_indirect_blocks() {
        let f = Ext2FixtureBuilder::new(1, 256)
            .with_free_blocks(64, 64)
            .build()
            .unwrap();
        let ext2 = &f.ext2;
        let ptrs = PTRS_PER_BLOCK as u32;
        let first_double_iblock = 12 + ptrs;
        let first_triple_iblock = 12 + ptrs + (1u32 << (ptrs.trailing_zeros() * 2));

        let mut block_ptr_tree = make_block_ptr_tree([0u32; RAW_BLOCK_PTRS_LEN], 0, &f.ext2);
        alloc_single_block(&mut block_ptr_tree, ext2, 12).unwrap();
        alloc_single_block(&mut block_ptr_tree, ext2, first_double_iblock).unwrap();
        alloc_single_block(&mut block_ptr_tree, ext2, first_triple_iblock).unwrap();
        assert_ne!(block_ptr_tree.raw_block_ptrs.block_ptrs[12], 0);
        assert_ne!(block_ptr_tree.raw_block_ptrs.block_ptrs[13], 0);
        assert_ne!(block_ptr_tree.raw_block_ptrs.block_ptrs[14], 0);

        block_ptr_tree.truncate_to_byte_len(ext2, 0);
        assert_eq!(block_ptr_tree.raw_block_ptrs.block_ptrs[12], 0);
        assert_eq!(block_ptr_tree.raw_block_ptrs.block_ptrs[13], 0);
        assert_eq!(block_ptr_tree.raw_block_ptrs.block_ptrs[14], 0);
        assert_eq!(block_ptr_tree.lookup_block(12).unwrap(), None);
        assert_eq!(
            block_ptr_tree.lookup_block(first_double_iblock).unwrap(),
            None
        );
        assert_eq!(
            block_ptr_tree.lookup_block(first_triple_iblock).unwrap(),
            None
        );
        assert_eq!(block_ptr_tree.raw_block_ptrs.sector_count, 0);
    }

    #[ktest]
    fn free_block_subtree_recursively_releases_blocks() {
        let f = Ext2FixtureBuilder::new(1, 256)
            .with_free_blocks(64, 64)
            .build()
            .unwrap();
        let ext2 = &f.ext2;

        let ptrs = PTRS_PER_BLOCK as u32;
        let first_triple_iblock = 12 + ptrs + (1u32 << (ptrs.trailing_zeros() * 2));

        let mut block_ptr_tree = make_block_ptr_tree([0u32; RAW_BLOCK_PTRS_LEN], 0, &f.ext2);
        alloc_single_block(&mut block_ptr_tree, ext2, first_triple_iblock).unwrap();
        let root = block_ptr_tree.raw_block_ptrs.block_ptrs[14];
        assert_ne!(root, 0);
        assert_eq!(
            block_ptr_tree.raw_block_ptrs.sector_count,
            SECTORS_PER_BLOCK * 4
        );

        let free_before = ext2.super_block().free_blocks_count();
        block_ptr_tree.free_block_subtree(ext2, root, 3);
        block_ptr_tree.raw_block_ptrs.block_ptrs[14] = 0;
        let free_after = ext2.super_block().free_blocks_count();

        assert_eq!(free_after - free_before, 4);
        assert_eq!(block_ptr_tree.raw_block_ptrs.sector_count, 0);
    }
}
