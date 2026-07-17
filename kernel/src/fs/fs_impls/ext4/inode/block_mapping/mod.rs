// SPDX-License-Identifier: MPL-2.0

//! Per-inode logical-to-physical block mapping.
//!
//! `BlockMapping` is the single dispatch point between an inode's file view
//! and the engine that maps its logical blocks to device blocks. The variant
//! is selected once, when the inode is loaded or created, and never changes
//! for the life of the inode. Every engine keeps its own interior lock; this
//! layer adds none, so the cross-layer lock order documented in
//! [`super`] is unchanged.

use super::{super::prelude::*, InodeDesc, RAW_BLOCK_PTRS_LEN, empty_extent_root};

mod extent;
mod indirect;

use self::{extent::ExtentManager, indirect::IndirectManager};
use super::super::fs::Ext4;

/// State of an allocated logical block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MapState {
    /// Backed by written data on disk.
    Written,
    /// Allocated but never written; reads as zeros.
    Unwritten,
}

/// The result of mapping a contiguous logical-block run.
///
/// This inherits the direct-I/O run classification role of ext2's old
/// `IoRange` (mapped vs. hole), generalized into this per-run `map_blocks`
/// form so the indirect and extent engines can share one interface and express
/// the extent engine's unwritten state -- the same shape as Linux's
/// `ext4_map_blocks`.
#[derive(Clone, Copy, Debug)]
pub(super) enum Mapping {
    /// A run backed by physical blocks.
    Mapped {
        pblock: Ext4Bid,
        len: u32,
        state: MapState,
    },
    /// A run with no physical allocation.
    Hole { len: u32 },
}

impl Mapping {
    /// Returns the starting physical block for an allocated run.
    pub(super) const fn pblock(&self) -> Option<Ext4Bid> {
        match self {
            Self::Mapped { pblock, .. } => Some(*pblock),
            Self::Hole { .. } => None,
        }
    }

    /// Returns the number of contiguous logical blocks this mapping describes.
    pub(super) const fn len(&self) -> u32 {
        match self {
            Self::Mapped { len, .. } | Self::Hole { len } => *len,
        }
    }

    #[cfg(ktest)]
    pub(super) const fn state(&self) -> Option<MapState> {
        match self {
            Self::Mapped { state, .. } => Some(*state),
            Self::Hole { .. } => None,
        }
    }

    /// Returns whether reading these blocks must return zeros without device I/O.
    pub(super) const fn reads_as_zeros(&self) -> bool {
        matches!(
            self,
            Self::Hole { .. }
                | Self::Mapped {
                    state: MapState::Unwritten,
                    ..
                }
        )
    }
}

/// A data-backed inode's block-mapping engine.
///
/// The variant follows the inode's `EXTENTS` flag: ext4-native inodes map
/// through an extent tree rooted inline in `i_block`, while ext2-style inodes
/// map through the classic direct/indirect pointer tree stored in the same
/// bytes. All engine state, including the interior lock, lives in the
/// variant; callers treat this enum as the engine.
pub(super) enum BlockMapping {
    Extent(ExtentManager),
    Indirect(IndirectManager),
}

impl BlockMapping {
    /// Builds the mapping engine for a data-backed inode from its descriptor.
    pub(super) fn new(desc: &InodeDesc, fs: Weak<Ext4>, npages: usize) -> Result<Self> {
        let root = *desc.raw_block();
        let sector_count = desc.sector_count();
        if desc.is_extent_based() {
            let manager = ExtentManager::new(root, sector_count, fs, npages)?;
            Ok(Self::Extent(manager))
        } else {
            let manager = IndirectManager::new(root, sector_count, fs, npages)?;
            Ok(Self::Indirect(manager))
        }
    }

    /// Builds a fresh, empty mapping with no blocks allocated, in the
    /// volume's format: extent volumes get an extent root, ext2-format
    /// volumes an indirect pointer array (the same rule `create_inode`
    /// follows for new inodes).
    ///
    /// Used by the fast-to-slow symlink switch, which replaces inline target
    /// bytes with a mapped data block; the inode's current `i_block` holds
    /// those raw bytes and must not be interpreted as a mapping root.
    pub(super) fn new_empty(fs: Weak<Ext4>, extent_based: bool, npages: usize) -> Result<Self> {
        if extent_based {
            Ok(Self::Extent(ExtentManager::new(
                empty_extent_root(),
                0,
                fs,
                npages,
            )?))
        } else {
            Ok(Self::Indirect(IndirectManager::new(
                [0u32; RAW_BLOCK_PTRS_LEN],
                0,
                fs,
                npages,
            )?))
        }
    }

    /// Maps logical block `iblock` to a physical run.
    ///
    /// The returned length spans from `iblock` to the end of the covering
    /// mapped (or hole) run, so callers can batch contiguous I/O.
    pub(super) fn map_blocks(&self, iblock: Iblock) -> Result<Mapping> {
        match self {
            Self::Extent(m) => m.map_blocks(iblock),
            Self::Indirect(m) => m.map_blocks(iblock),
        }
    }

    /// Allocates data blocks for every hole in `[start_iblock, end_iblock)`
    /// and records them in the mapping.
    pub(super) fn ensure_allocated(&self, start_iblock: Iblock, end_iblock: Iblock) -> Result<()> {
        match self {
            Self::Extent(m) => m.ensure_allocated(start_iblock, end_iblock),
            Self::Indirect(m) => {
                m.allocate_range_blocks(start_iblock as usize, end_iblock as usize)
            }
        }
    }

    /// Preallocates blocks for every hole in `[start_iblock, end_iblock)`
    /// without touching file size. Backs `fallocate`.
    ///
    /// The extent engine records the new blocks as unwritten extents (they
    /// read as zeros until written); the indirect engine has no unwritten
    /// state, so it allocates real zeroed blocks (ext2 semantics).
    pub(super) fn preallocate(&self, start_iblock: Iblock, end_iblock: Iblock) -> Result<()> {
        match self {
            Self::Extent(m) => m.preallocate(start_iblock, end_iblock),
            Self::Indirect(m) => {
                m.allocate_range_blocks(start_iblock as usize, end_iblock as usize)
            }
        }
    }

    /// Frees every data and engine-metadata block mapping a logical region at
    /// or beyond `new_size` bytes, updating `i_blocks`.
    pub(super) fn truncate_to_byte_len(&self, new_size: usize) -> Result<()> {
        match self {
            Self::Extent(m) => m.truncate_to_byte_len(new_size),
            Self::Indirect(m) => {
                // The indirect engine truncates best-effort (errors are logged
                // and the freed prefix is kept); the inode reclaim path relies
                // on truncation never failing.
                m.truncate_to_byte_len(new_size);
                Ok(())
            }
        }
    }

    /// Returns a copy of the inode's 60-byte `i_block` as the engine persists
    /// it (the extent-tree root or the pointer array).
    pub(super) fn root_snapshot(&self) -> [u32; RAW_BLOCK_PTRS_LEN] {
        match self {
            Self::Extent(m) => m.root_snapshot(),
            Self::Indirect(m) => m.root_snapshot(),
        }
    }

    /// Returns the inode's `i_blocks` (512-byte sectors) accounting.
    pub(super) fn sector_count(&self) -> u64 {
        match self {
            Self::Extent(m) => m.sector_count(),
            Self::Indirect(m) => m.sector_count(),
        }
    }

    /// Flushes engine-internal metadata blocks to the device.
    ///
    /// Called before the inode itself is written back, so the on-disk inode
    /// never references engine metadata that is not yet durable. The extent
    /// engine writes its interior tree nodes through at tree-operation time,
    /// so it has nothing to flush here; the indirect engine caches dirty
    /// indirect blocks until this point.
    pub(super) fn sync_meta(&self) -> Result<()> {
        match self {
            Self::Extent(_) => Ok(()),
            Self::Indirect(m) => m.sync_indirect_blocks(),
        }
    }

    /// Returns whether the mapping or `i_blocks` changed since last writeback.
    pub(super) fn is_dirty(&self) -> bool {
        match self {
            Self::Extent(m) => m.is_dirty(),
            Self::Indirect(m) => m.is_dirty(),
        }
    }

    /// Clears the dirty flag after a successful inode writeback.
    pub(super) fn clear_dirty(&self) {
        match self {
            Self::Extent(m) => m.clear_dirty(),
            Self::Indirect(m) => m.clear_dirty(),
        }
    }

    /// Updates the cached page-cache capacity bound.
    pub(super) fn set_npages(&self, npages: usize) {
        match self {
            Self::Extent(m) => m.set_npages(npages),
            Self::Indirect(m) => m.set_npages(npages),
        }
    }

    /// Returns the largest byte size a file with this mapping type can reach.
    ///
    /// An extent maps a 32-bit logical block index, so an extent-mapped file
    /// spans at most `2^32 - 1` blocks, clamped to `i64::MAX` (the VFS size
    /// limit); the mutation path has a stricter practical limit because it
    /// only rebuilds extent trees up to depth 1. Indirect-mapped files follow
    /// the ext2 geometry and `i_blocks` accounting limits.
    pub(super) fn max_file_size(&self) -> usize {
        match self {
            Self::Extent(_) => {
                let by_blocks = (u32::MAX as u64) * BLOCK_SIZE as u64;
                usize::try_from(by_blocks.min(i64::MAX as u64)).unwrap_or(usize::MAX)
            }
            Self::Indirect(_) => indirect::max_file_size(),
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
            Self::Extent(m) => m.submit_read_bio(idx, bio_segment, complete_fn, io_batch),
            Self::Indirect(m) => m.submit_read_bio(idx, bio_segment, complete_fn, io_batch),
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
            Self::Extent(m) => m.submit_write_bio(idx, bio_segment, complete_fn, io_batch),
            Self::Indirect(m) => m.submit_write_bio(idx, bio_segment, complete_fn, io_batch),
        }
    }
}
