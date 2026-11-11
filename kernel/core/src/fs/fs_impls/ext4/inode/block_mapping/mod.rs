// SPDX-License-Identifier: MPL-2.0

//! Per-inode logical-to-physical block mapping.
//!
//! [`BlockMapping`] coordinates mapping synchronization and page-count bounds,
//! and holds the filesystem reference for page-cache I/O.
//! Each format-specific tree manages its mapping state and runtime caches.
//! Callers use one interface for classic block pointers and EXT4 extents.
//!
//! Page-cache I/O releases the mapping lock before data BIO submission.
//! Direct-I/O range iteration keeps a read guard while its mapping plan is
//! consumed, giving both mapping formats the same stable-iteration behavior.
//! The inode lock remains responsible for coordinating mapping changes with
//! inode size and metadata updates.

#![short_vis_path::add(inode)]

mod block_manager;
pub(super) mod extent;

use core::sync::atomic::{AtomicUsize, Ordering};

use aster_block::bio::BioCompleteFn;
use device_id::{decode_device_numbers, encode_device_numbers};

use self::{
    block_manager::{BlockPtrTree, ResolvedBlockRange},
    extent::{AllocationMode, ExtentTree},
};
use super::{
    Iblock, RAW_BLOCK_PTRS_LEN,
    io_range::{Backing, IoRange, IoRangeIter},
};
use crate::{
    fs::ext4::{fs::Ext4, prelude::*},
    vm::page_cache::PageRun,
};

/// A data-backed inode's logical-to-physical block mapping.
///
/// Owns synchronization and the page-cache backend interface for both mapping
/// formats. The selected tree and its mutable state reside in [`MappingTreeType`].
#[derive(Debug)]
pub(super) struct BlockMapping {
    manager: RwMutex<MappingTreeType>,
    npages: AtomicUsize,
    fs: Weak<Ext4>,
}

/// The selected mapping-tree representation and its mutable state.
///
/// Dispatches tree operations while [`BlockMapping`] owns synchronization and
/// the page-cache backend interface.
#[derive(Debug)]
pub(super) enum MappingTreeType {
    /// An EXT4 extent tree rooted in the inode's `i_block` field.
    ExtentTree(ExtentTree),
    /// A classic EXT direct/indirect block-pointer tree.
    BlockPtrTree(BlockPtrTree),
}

/// On-disk mapping state mirrored from an inode.
#[derive(Clone, Copy, Debug)]
pub(in inode) struct RawBlockPtrs {
    pub sector_count: u32,
    pub block_ptrs: [u32; RAW_BLOCK_PTRS_LEN],
}

impl RawBlockPtrs {
    pub(in inode) fn new(sector_count: u32, block_ptrs: [u32; RAW_BLOCK_PTRS_LEN]) -> Self {
        Self {
            sector_count,
            block_ptrs,
        }
    }

    pub(in inode) fn read_device_id(&self) -> u64 {
        let (major, minor) = if self.block_ptrs[0] != 0 {
            let encoded = self.block_ptrs[0];
            (((encoded >> 8) & 0xff), (encoded & 0xff))
        } else {
            let encoded = self.block_ptrs[1];
            (
                ((encoded & 0xfff00) >> 8),
                ((encoded & 0xff) | ((encoded >> 12) & 0xfff00)),
            )
        };
        encode_device_numbers(major, minor)
    }

    pub(in inode) fn write_device_id(&mut self, device_id: u64) {
        let (major, minor) = decode_device_numbers(device_id);
        if major < 256 && minor < 256 {
            self.block_ptrs[0] = (major << 8) | minor;
            self.block_ptrs[1] = 0;
        } else {
            self.block_ptrs[0] = 0;
            self.block_ptrs[1] = (minor & 0xff) | (major << 8) | ((minor & !0xff) << 12);
            self.block_ptrs[2] = 0;
        }
    }
}

/// Per-iterator format state, retained by the same guard as the tree.
pub(super) enum MappingRangeCursor {
    Indirect,
    Extent(extent::ExtentRangeCursor),
}

/// A candidate built while the owning inode is write-locked.
/// The caller must retain that lock until commit or discard, excluding mapping
/// changes while the mapping lock is released for page-cache preparation.
#[must_use]
pub(super) struct PreparedTruncate<'a> {
    mapping: &'a BlockMapping,
    fs: &'a Ext4,
    new_size: usize,
    extent_update: Option<extent::PreparedUpdate<'a>>,
}

impl PreparedTruncate<'_> {
    /// Publishes the candidate into its original mapping while the inode stays
    /// write-locked. An error means block release failed after the mapping was
    /// committed; callers must update inode size before propagating the error.
    pub(super) fn commit(self) -> Result<()> {
        match &mut *self.mapping.manager.write() {
            MappingTreeType::ExtentTree(tree) => tree.commit_truncate(self.extent_update),
            MappingTreeType::BlockPtrTree(tree) => {
                tree.truncate_to_byte_len(self.fs, self.new_size);
                Ok(())
            }
        }
    }
}

impl MappingTreeType {
    pub(super) fn next_io_range(
        &self,
        fs: &Ext4,
        range: &mut Range<Iblock>,
        cursor: &mut MappingRangeCursor,
    ) -> Result<Option<IoRange>> {
        if range.start >= range.end {
            return Ok(None);
        }
        let start = range.start;
        let max_blocks = range.len() as u32;
        match self {
            Self::BlockPtrTree(tree) => {
                let mapped = tree.lookup_block_range(start, max_blocks)?;
                if mapped.is_empty() {
                    let hole_len = tree.approx_hole_blocks(start, max_blocks)?;
                    debug_assert!(hole_len > 0);
                    range.start += hole_len;
                    Ok(Some(IoRange {
                        logical: start..range.start,
                        backing: Backing::Hole,
                    }))
                } else {
                    range.start += mapped.len() as u32;
                    Ok(Some(IoRange {
                        logical: start..range.start,
                        backing: Backing::Written {
                            physical_start: mapped.start,
                        },
                    }))
                }
            }
            Self::ExtentTree(tree) => {
                if !matches!(cursor, MappingRangeCursor::Extent(_)) {
                    *cursor = MappingRangeCursor::Extent(extent::ExtentRangeCursor::default());
                }
                let MappingRangeCursor::Extent(cursor) = cursor else {
                    unreachable!()
                };
                tree.next_io_range(fs, range, cursor).map(Some)
            }
        }
    }
}

impl BlockMapping {
    /// Creates an indirect mapping from the inode's pointer array.
    pub(super) fn new_indirect(
        raw_block_ptrs: RawBlockPtrs,
        fs: Weak<Ext4>,
        npages: usize,
    ) -> Self {
        Self {
            manager: RwMutex::new(MappingTreeType::BlockPtrTree(BlockPtrTree::new(
                raw_block_ptrs,
                fs.clone(),
            ))),
            npages: AtomicUsize::new(npages),
            fs,
        }
    }

    /// Creates an extent mapping from a validated on-disk `i_block` root.
    pub(super) fn new_extent(
        raw_block_ptrs: RawBlockPtrs,
        fs: Weak<Ext4>,
        npages: usize,
    ) -> Result<Self> {
        Ok(Self {
            manager: RwMutex::new(MappingTreeType::ExtentTree(ExtentTree::new(
                raw_block_ptrs,
            )?)),
            npages: AtomicUsize::new(npages),
            fs,
        })
    }

    /// Returns the mapping root and sector count to store in the raw inode.
    pub(super) fn raw_block_ptrs(&self) -> RawBlockPtrs {
        let manager = self.manager.read();
        match &*manager {
            MappingTreeType::ExtentTree(state) => state.raw_block_ptrs(),
            MappingTreeType::BlockPtrTree(tree) => *tree.raw_block_ptrs(),
        }
    }

    /// Returns whether the inode-resident mapping metadata needs writeback.
    pub(super) fn is_inode_dirty(&self) -> bool {
        let manager = self.manager.read();
        match &*manager {
            MappingTreeType::ExtentTree(tree) => tree.is_inode_dirty(),
            MappingTreeType::BlockPtrTree(tree) => tree.is_inode_dirty(),
        }
    }

    /// Returns whether inode-resident or external mapping metadata needs writeback.
    ///
    /// External metadata includes extent nodes or indirect blocks, depending
    /// on the mapping format.
    pub(super) fn has_dirty_metadata(&self) -> bool {
        if self.is_inode_dirty() {
            return true;
        }
        let manager = self.manager.read();
        match &*manager {
            MappingTreeType::ExtentTree(tree) => tree.has_dirty_metadata(),
            MappingTreeType::BlockPtrTree(tree) => tree.has_dirty_metadata(),
        }
    }

    /// Clears only the dirty flag for the inode-resident mapping metadata.
    ///
    /// External extent nodes and indirect blocks are unaffected; they are
    /// written back separately by `sync_metadata`.
    pub(super) fn clear_inode_dirty(&self) {
        let mut manager = self.manager.write();
        match &mut *manager {
            MappingTreeType::ExtentTree(state) => state.clear_inode_dirty(),
            MappingTreeType::BlockPtrTree(tree) => tree.clear_inode_dirty(),
        }
    }

    /// Returns mapped physical runs and holes over a logical block range.
    pub(super) fn iter_io_ranges(&self, block_range: Range<Iblock>) -> Result<IoRangeIter<'_>> {
        Ok(IoRangeIter::new(
            block_range,
            self.manager.read(),
            self.fs()?,
        ))
    }

    /// Prepares truncation without changing mappings or releasing blocks.
    /// The owning inode must remain write-locked until commit or discard.
    pub(super) fn prepare_truncate<'a>(
        &'a self,
        fs: &'a Ext4,
        new_size: usize,
    ) -> Result<PreparedTruncate<'a>> {
        let extent_update = match &*self.manager.read() {
            MappingTreeType::ExtentTree(tree) => tree.prepare_truncate(fs, new_size)?,
            MappingTreeType::BlockPtrTree(_) => None,
        };
        Ok(PreparedTruncate {
            mapping: self,
            fs,
            new_size,
            extent_update,
        })
    }

    /// Removes mappings beyond the block containing `new_size`.
    ///
    /// On error, mappings may already have been removed while some blocks still
    /// await release. The caller must not assume the original mappings remain
    /// intact or free the inode as though truncation had completed successfully.
    ///
    /// This method does not update the inode's file size or page cache.
    /// Use [`Self::prepare_truncate`] to prepare mapping changes before updating
    /// the page cache, then commit the changes and update the inode's file size.
    /// Block-pointer truncation logs internal errors without propagating them.
    pub(super) fn truncate_to_byte_len(&self, new_size: usize) -> Result<()> {
        let fs = self.fs()?;
        let mut manager = self.manager.write();
        match &mut *manager {
            MappingTreeType::ExtentTree(tree) => tree.truncate_to_byte_len(&fs, new_size),
            MappingTreeType::BlockPtrTree(tree) => {
                tree.truncate_to_byte_len(&fs, new_size);
                Ok(())
            }
        }
    }

    /// Writes mapping metadata stored outside the inode to the block device.
    pub(super) fn sync_metadata(&self) -> Result<()> {
        let manager = self.manager.read();
        match &*manager {
            MappingTreeType::ExtentTree(tree) => tree.sync_metadata(self.fs()?.as_ref()),
            MappingTreeType::BlockPtrTree(tree) => tree.sync_indirect_blocks(),
        }
    }

    /// Reserves direct-write holes without exposing uninitialized data.
    pub(super) fn allocate_direct_blocks(&self, start: usize, end: usize) -> Result<()> {
        let fs = self.fs()?;
        {
            let mut manager = self.manager.write();
            if let MappingTreeType::ExtentTree(tree) = &mut *manager {
                return tree.allocate_range(&fs, start, end, AllocationMode::Unwritten);
            }
        }
        self.allocate_range_blocks(start, end)
    }

    pub(super) fn mark_direct_written(&self, start: usize, end: usize) -> Result<()> {
        let fs = self.fs()?;
        let mut manager = self.manager.write();
        if let MappingTreeType::ExtentTree(tree) = &mut *manager {
            tree.mark_written(&fs, start, end)?;
        }
        Ok(())
    }

    /// Ensures that every logical block in `[start_block, end_block)` is mapped.
    pub(super) fn allocate_range_blocks(&self, start_block: usize, end_block: usize) -> Result<()> {
        let fs = self.fs()?;
        let mut manager = self.manager.write();
        if let MappingTreeType::ExtentTree(state) = &mut *manager {
            return state.allocate_range_blocks(&fs, start_block, end_block);
        }
        let MappingTreeType::BlockPtrTree(tree) = &mut *manager else {
            unreachable!()
        };
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
        let fs = self.fs()?;
        let manager = self.manager.read();
        match &*manager {
            MappingTreeType::ExtentTree(state) => state.lookup_readable_block(&fs, iblock),
            MappingTreeType::BlockPtrTree(tree) => tree.lookup_block(iblock),
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

    fn submit_read_page_run(&self, mut pages: PageRun<'_>, io_batch: &mut IoBatch) -> Result<()> {
        let start_idx = pages.start_idx();
        let end_idx = start_idx + pages.len();
        if end_idx > self.npages.load(Ordering::Acquire) {
            return_errno_with_message!(Errno::EINVAL, "invalid read size");
        }

        let fs = self.fs()?;
        let mut io_ranges = self.iter_io_ranges(start_idx as Iblock..end_idx as Iblock)?;
        while let Some(io_range) = io_ranges.next()? {
            match io_range.backing {
                Backing::Written { physical_start } => {
                    let range_len = io_range.logical.len();
                    pages.submit_contiguous_read_pages(
                        fs.block_device(),
                        Bid::new(physical_start as u64),
                        range_len,
                        io_batch,
                    )?;
                }
                Backing::Hole | Backing::Unwritten { .. } => {
                    for locked_page in pages.by_ref().take(io_range.logical.len()) {
                        locked_page.fill_zeros(0, PAGE_SIZE).unwrap();
                        locked_page.set_up_to_date();
                    }
                }
            }
        }
        Ok(())
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

        if self.lookup_block(iblock)?.is_none() {
            self.allocate_range_blocks(idx, idx + bio_segment.nblocks())?;
        }
        let bid = self
            .lookup_block(iblock)?
            .ok_or_else(|| Error::with_message(Errno::EIO, "block allocation produced a hole"))?;

        fs.write_blocks_async(bid, bio_segment, Some(complete_fn), io_batch)
    }

    fn submit_write_page_run(&self, mut pages: PageRun<'_>, io_batch: &mut IoBatch) -> Result<()> {
        let start_idx = pages.start_idx();
        let end_idx = start_idx + pages.len();
        if end_idx > self.npages.load(Ordering::Acquire) {
            return_errno_with_message!(Errno::EINVAL, "invalid write size");
        }

        self.allocate_range_blocks(start_idx, end_idx)?;
        let fs = self.fs()?;
        let mut io_ranges = self.iter_io_ranges(start_idx as Iblock..end_idx as Iblock)?;
        while let Some(io_range) = io_ranges.next()? {
            match io_range.backing {
                Backing::Written { physical_start } => {
                    let range_len = io_range.logical.len();
                    pages.submit_contiguous_write_pages(
                        fs.block_device(),
                        Bid::new(physical_start as u64),
                        range_len,
                        io_batch,
                    )?;
                }
                Backing::Hole | Backing::Unwritten { .. } => {
                    return_errno_with_message!(
                        Errno::EIO,
                        "unexpected hole after writeback allocation"
                    );
                }
            }
        }
        Ok(())
    }
}
