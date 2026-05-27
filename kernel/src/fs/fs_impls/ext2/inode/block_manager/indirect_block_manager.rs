// SPDX-License-Identifier: MPL-2.0

//! Inode-local cache for ext2 indirect metadata blocks.

use hashbrown::HashMap;

use crate::fs::fs_impls::ext2::{Ext2, prelude::*};

/// Inode-local cache for ext2 indirect metadata blocks.
///
/// Resident indirect blocks belong to exactly one inode and are owned by the
/// corresponding `BlockPtrTree`. Dirty indirect blocks remain associated with
/// that inode until they are synced or removed.
///
/// # Invariants
///
/// - Cached blocks are never shared across inodes.
/// - Dirty blocks must remain reachable until they are either written back or
///   discarded as part of freeing their owning subtree.
#[derive(Debug)]
pub(super) struct IndirectBlockManager {
    cache: HashMap<Ext2Bid, IndirectBlock>,
    fs: Weak<Ext2>,
}

impl IndirectBlockManager {
    /// Creates a new indirect block manager.
    pub(super) fn new(fs: Weak<Ext2>) -> Self {
        Self {
            cache: HashMap::new(),
            fs,
        }
    }

    /// Returns a shared reference to the indirect block at `bid`, loading it if needed.
    pub(super) fn find(&mut self, bid: Ext2Bid) -> Result<&IndirectBlock> {
        self.ensure_cached(bid)?;
        Ok(self
            .cache
            .get(&bid)
            .expect("indirect block must be cached after ensure_cached"))
    }

    /// Returns a mutable reference to the indirect block at `bid`, loading it if needed.
    pub(super) fn find_mut(&mut self, bid: Ext2Bid) -> Result<&mut IndirectBlock> {
        self.ensure_cached(bid)?;
        Ok(self
            .cache
            .get_mut(&bid)
            .expect("indirect block must be cached after ensure_cached"))
    }

    /// Reads all non-zero child block pointers from an indirect block.
    pub(super) fn read_child_bids(&mut self, bid: Ext2Bid) -> Result<Vec<Ext2Bid>> {
        let ptrs_per_block = BLOCK_SIZE / size_of::<u32>();
        let block = self.find(bid)?;
        let mut child_bids = Vec::new();
        for idx in 0..ptrs_per_block {
            match block.read_bid(idx) {
                Ok(0) => continue,
                Ok(child_bid) => child_bids.push(child_bid),
                Err(_) => break,
            }
        }
        Ok(child_bids)
    }

    /// Inserts an indirect block into the cache.
    pub(super) fn insert(&mut self, block: IndirectBlock) -> Result<()> {
        self.cache.insert(block.bid(), block);
        Ok(())
    }

    /// Removes an indirect block from the cache without writing it back.
    pub(super) fn remove(&mut self, bid: Ext2Bid) -> Option<IndirectBlock> {
        self.cache.remove(&bid)
    }

    /// Writes all dirty cached indirect blocks back to the device.
    pub(super) fn sync(&mut self) -> Result<()> {
        let fs = self.fs()?;
        let dirty_bids: Vec<Ext2Bid> = self
            .cache
            .iter()
            .filter_map(|(bid, block)| block.is_dirty().then_some(*bid))
            .collect();

        // TODO: do this flush job in best effort (skip the failed flush).
        for bid in dirty_bids {
            let block = self
                .cache
                .get_mut(&bid)
                .expect("dirty indirect block key must still be cached");
            Self::write_block(&fs, block)?;
        }

        Ok(())
    }

    fn ensure_cached(&mut self, bid: Ext2Bid) -> Result<()> {
        if !self.cache.contains_key(&bid) {
            let block = self.load_block(bid)?;
            self.cache.insert(bid, block);
        }
        Ok(())
    }

    fn load_block(&self, bid: Ext2Bid) -> Result<IndirectBlock> {
        let fs = self.fs()?;
        let mut block = IndirectBlock::alloc_uninit()?;
        block.set_bid(bid);
        let bio_segment = BioSegment::new_from_segment(
            Segment::<()>::from(block.frame().clone()).into(),
            BioDirection::FromDevice,
        );
        fs.read_blocks(bid, bio_segment)
            .map_err(|_| Error::with_message(Errno::EIO, "failed to submit indirect block read"))?;
        block.mark_clean();
        Ok(block)
    }

    fn write_block(fs: &Arc<Ext2>, block: &mut IndirectBlock) -> Result<()> {
        if !block.is_dirty() {
            return Ok(());
        }

        let bio_segment = BioSegment::new_from_segment(
            Segment::<()>::from(block.frame().clone()).into(),
            BioDirection::ToDevice,
        );
        fs.write_blocks(block.bid(), bio_segment).map_err(|_| {
            Error::with_message(Errno::EIO, "failed to submit indirect block writeback")
        })?;

        block.mark_clean();
        Ok(())
    }

    fn fs(&self) -> Result<Arc<Ext2>> {
        self.fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "filesystem already dropped"))
    }
}

/// One resident indirect metadata block.
///
/// Each `IndirectBlock` is a single page-frame that holds one ext2 indirect
/// metadata block (i.e. an array of `u32` block-number pointers). It tracks
/// its own device block address (`bid`) and a dirty flag so that write-back
/// can be skipped for clean entries.
#[derive(Debug)]
pub(super) struct IndirectBlock {
    block: Frame<()>,
    bid: Ext2Bid,
    dirty: bool,
}

impl IndirectBlock {
    /// Allocates an uninitialized indirect block frame.
    pub(super) fn alloc_uninit() -> Result<Self> {
        Ok(Self {
            block: FrameAllocOptions::new().zeroed(false).alloc_frame()?,
            bid: 0,
            dirty: false,
        })
    }

    /// Allocates a zeroed indirect block and marks it dirty.
    pub(super) fn alloc_new(bid: Ext2Bid) -> Result<Self> {
        Ok(Self {
            block: FrameAllocOptions::new().alloc_frame()?,
            bid,
            dirty: true,
        })
    }

    /// Reads the block pointer at slot `idx`.
    pub(super) fn read_bid(&self, idx: usize) -> Result<Ext2Bid> {
        let slot_offset = self.slot_offset(idx);
        self.block
            .read_val(slot_offset)
            .map_err(|_| Error::with_message(Errno::EIO, "failed to read indirect pointer"))
    }

    /// Writes a block pointer at slot `idx` and marks the block dirty.
    pub(super) fn write_bid(&mut self, idx: usize, bid: Ext2Bid) -> Result<()> {
        let slot_offset = self.slot_offset(idx);
        self.block
            .write_val(slot_offset, &bid)
            .map_err(|_| Error::with_message(Errno::EIO, "failed to write indirect pointer"))?;
        self.dirty = true;
        Ok(())
    }

    /// Returns whether the block has been modified since the last write-back.
    pub(super) fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Returns the device block address of this indirect block.
    pub(super) fn bid(&self) -> Ext2Bid {
        self.bid
    }

    fn frame(&self) -> &Frame<()> {
        &self.block
    }

    fn mark_clean(&mut self) {
        self.dirty = false;
    }

    fn set_bid(&mut self, bid: Ext2Bid) {
        self.bid = bid;
    }

    fn slot_offset(&self, idx: usize) -> usize {
        debug_assert!(idx < BLOCK_SIZE / size_of::<Ext2Bid>());
        idx * size_of::<Ext2Bid>()
    }
}
