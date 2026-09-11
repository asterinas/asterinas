// SPDX-License-Identifier: MPL-2.0

//! Per-inode logical-to-physical block mapping.
//!
//! This module is the dispatch point between inode I/O and its on-disk block
//! mapping format. Only the classic direct/indirect format is connected here;
//! extent support is added by later commits.

use aster_block::bio::BioCompleteFn;

use super::{
    Iblock,
    block_manager::{BlockPtrTree, InodeBlockManager, RawBlockPtrs},
    io_range::IoRangeIter,
};
use crate::fs::ext4::{fs::Ext4, prelude::*};

pub(super) mod extent;

use self::extent::ExtentManager;

/// A data-backed inode's block-mapping engine.
#[derive(Debug)]
pub(super) enum BlockMapping {
    Extent(ExtentManager),
    Indirect(InodeBlockManager),
}

impl BlockMapping {
    /// Creates an indirect mapping from the inode's pointer array.
    pub(super) fn new_indirect(
        raw_block_ptrs: RawBlockPtrs,
        fs: Weak<Ext4>,
        npages: usize,
    ) -> Self {
        let tree = BlockPtrTree::new(raw_block_ptrs, fs.clone());
        Self::Indirect(InodeBlockManager::new(tree, fs, npages))
    }

    pub(super) fn new_extent(
        raw_block_ptrs: RawBlockPtrs,
        fs: Weak<Ext4>,
        npages: usize,
    ) -> Result<Self> {
        Ok(Self::Extent(ExtentManager::new(
            raw_block_ptrs.block_ptrs,
            raw_block_ptrs.sector_count,
            fs,
            npages,
        )?))
    }

    pub(super) fn raw_block_ptrs(&self) -> RawBlockPtrs {
        match self {
            Self::Extent(mapping) => mapping.raw_block_ptrs(),
            Self::Indirect(mapping) => mapping.raw_block_ptrs(),
        }
    }

    pub(super) fn is_dirty(&self) -> bool {
        match self {
            Self::Extent(mapping) => mapping.is_dirty(),
            Self::Indirect(mapping) => mapping.is_dirty(),
        }
    }

    pub(super) fn clear_dirty(&self) {
        match self {
            Self::Extent(mapping) => mapping.clear_dirty(),
            Self::Indirect(mapping) => mapping.clear_dirty(),
        }
    }

    pub(super) fn iter_io_ranges(&self, block_range: Range<Iblock>) -> IoRangeIter<'_> {
        match self {
            Self::Extent(mapping) => IoRangeIter::new_extent(block_range, mapping),
            Self::Indirect(mapping) => mapping.iter_io_ranges(block_range),
        }
    }

    pub(super) fn truncate_to_byte_len(&self, new_size: usize) -> Result<()> {
        match self {
            Self::Extent(mapping) => mapping.truncate_to_byte_len(new_size),
            Self::Indirect(mapping) => {
                mapping.truncate_to_byte_len(new_size);
                Ok(())
            }
        }
    }

    pub(super) fn sync_metadata(&self) -> Result<()> {
        match self {
            Self::Extent(_) => Ok(()),
            Self::Indirect(mapping) => mapping.sync_indirect_blocks(),
        }
    }

    pub(super) fn allocate_range_blocks(&self, start_block: usize, end_block: usize) -> Result<()> {
        match self {
            Self::Extent(mapping) => mapping.allocate_range_blocks(start_block, end_block),
            Self::Indirect(mapping) => mapping.allocate_range_blocks(start_block, end_block),
        }
    }

    pub(super) fn set_npages(&self, npages: usize) {
        match self {
            Self::Extent(mapping) => mapping.set_npages(npages),
            Self::Indirect(mapping) => mapping.set_npages(npages),
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
        match self {
            Self::Extent(mapping) => {
                mapping.submit_read_bio(idx, bio_segment, complete_fn, io_batch)
            }
            Self::Indirect(mapping) => {
                mapping.submit_read_bio(idx, bio_segment, complete_fn, io_batch)
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
        match self {
            Self::Extent(mapping) => {
                mapping.submit_write_bio(idx, bio_segment, complete_fn, io_batch)
            }
            Self::Indirect(mapping) => {
                mapping.submit_write_bio(idx, bio_segment, complete_fn, io_batch)
            }
        }
    }
}
