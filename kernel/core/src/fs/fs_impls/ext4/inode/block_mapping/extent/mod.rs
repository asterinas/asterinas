// SPDX-License-Identifier: MPL-2.0

//! Per-inode EXT4 extent mapping.

use self::{
    node::{Extent, ExtentNode},
    path::ExtentRun,
};
use super::RawBlockPtrs;
use crate::fs::fs_impls::ext4::{fs::Ext4, inode::RAW_BLOCK_PTRS_LEN, prelude::*};

mod cache;
mod mutation;
mod node;
mod path;
mod status;
pub(super) use path::ExtentRangeCursor;

/// An inode's EXT4 extent mapping and sector accounting.
///
/// The root occupies the inode's 60-byte `i_block` field. A depth-zero root
/// contains extents directly; a deeper root contains indexes to child nodes.
/// Roots and external indexes can address up to five external levels:
///
/// ```text
/// inode.i_block
/// +------------------+     depth = 0
/// | header | extents |--------------------------> data blocks
/// +------------------+
///
/// +------------------+     depth >= 1    indexes / leaf blocks
/// | header | indexes |--------------> +------------------+
/// +------------------+                | header | extents |--> data blocks
///                                     +------------------+
/// ```
///
/// Extents are ordered and do not overlap logically. Referenced physical
/// ranges must lie in data space, outside filesystem system metadata.
/// New data blocks are zeroed or kept unwritten until initialized.
/// `raw_block_ptrs` holds the root and the count of data and tree blocks in
/// 512-byte sectors, with dirty tracking independent of file data.
/// The parent [`BlockMapping`](super::BlockMapping) lock protects queries and
/// serializes mapping changes, block allocation/release, and accounting updates.
///
/// Local updates prepare new resources before accepting the root and dirty
/// nodes under the mapping write lock. Failed preparation preserves the old
/// mapping; failed cleanup after acceptance retains unfinished frees in EXT.
/// External nodes are synced before the root enters inode-table page cache.
/// Written/unwritten extents and 32-bit physical addresses are supported;
/// metadata updates are neither journaled nor crash-atomic.
#[derive(Debug)]
pub(in crate::fs::fs_impls::ext4::inode) struct ExtentTree {
    raw_block_ptrs: Dirty<RawBlockPtrs>,
    node_cache: cache::ExtentNodeCache,
    root_cache: Mutex<Option<Arc<ExtentNode>>>,
    lookup_cache: Mutex<status::LookupCache>,
}

/// The initialization guarantee for newly allocated data blocks.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AllocationMode {
    /// Zero blocks before publishing them as readable mappings.
    Initialized,
    /// Publish unwritten mappings until direct data I/O completes.
    Unwritten,
}

impl ExtentTree {
    /// Creates an extent mapping from an inode-resident root and sector count.
    ///
    /// This validates the inline root. External nodes are loaded and validated
    /// when an operation traverses or rewrites the tree.
    pub(super) fn new(raw_block_ptrs: RawBlockPtrs) -> Result<Self> {
        Self::validate_root(&raw_block_ptrs.block_ptrs)?;
        Ok(Self {
            raw_block_ptrs: Dirty::new(raw_block_ptrs),
            node_cache: cache::ExtentNodeCache::new(),
            root_cache: Mutex::new(None),
            lookup_cache: Mutex::new(status::LookupCache::default()),
        })
    }

    /// Resolves one logical block to a physical block.
    ///
    /// Returns `None` when the logical block lies in a hole.
    pub(super) fn map_block(&self, fs: &Ext4, iblock: Iblock) -> Result<Option<Ext4Bid>> {
        Ok(self
            .find(fs, iblock)?
            .filter(|extent| !extent.is_unwritten())
            .map(|extent| extent.physical_block(iblock)))
    }

    /// Returns the extent root and sector count to store in the raw inode.
    pub(super) fn raw_block_ptrs(&self) -> RawBlockPtrs {
        *self.raw_block_ptrs
    }

    /// Returns whether the root or external extent nodes need writeback.
    pub(super) fn is_dirty(&self) -> bool {
        self.raw_block_ptrs.is_dirty() || self.node_cache.is_dirty()
    }

    /// Marks the inode-resident extent metadata as written back.
    pub(super) fn clear_dirty(&mut self) {
        self.raw_block_ptrs.clear_dirty();
    }

    /// Writes dirty external nodes from leaves towards the root.
    pub(super) fn sync_metadata(&self, fs: &Ext4) -> Result<()> {
        self.node_cache.sync(fs)
    }
}

const SECTORS_PER_BLOCK: u32 = (BLOCK_SIZE / SECTOR_SIZE) as u32;

pub(in crate::fs::fs_impls::ext4::inode) const fn empty_extent_root() -> [u32; RAW_BLOCK_PTRS_LEN] {
    let magic = node::EXTENT_MAGIC.to_ne_bytes();
    let max = 4u16.to_ne_bytes();
    let mut root = [0u32; RAW_BLOCK_PTRS_LEN];
    root[0] = u32::from_ne_bytes([magic[0], magic[1], 0, 0]);
    root[1] = u32::from_ne_bytes([max[0], max[1], 0, 0]);
    root
}

pub(in crate::fs::fs_impls::ext4::inode) fn validate_extent_root(
    root: [u32; RAW_BLOCK_PTRS_LEN],
) -> Result<()> {
    ExtentTree::validate_root(&root)
}

#[cfg(ktest)]
mod tests;

impl ExtentTree {
    fn validate_root(root: &[u32; RAW_BLOCK_PTRS_LEN]) -> Result<()> {
        ExtentNode::parse(root.as_bytes(), None)?;
        Ok(())
    }

    fn find(&self, fs: &Ext4, iblock: Iblock) -> Result<Option<Extent>> {
        match self.lookup_run(fs, iblock, 1u64 << 32)? {
            ExtentRun::Mapped { extent, .. } => Ok(Some(extent)),
            ExtentRun::Hole(_) => Ok(None),
        }
    }

    /// Validates a physical range referenced by an inode extent tree.
    fn validate_extent_block_range(fs: &Ext4, start: Ext4Bid, count: u32) -> Result<()> {
        let sb = fs.super_block();
        if !sb.is_data_block_valid(start, count) {
            return_errno_with_message!(Errno::EUCLEAN, "extent block range is outside filesystem");
        }

        let end = start
            .checked_add(count)
            .ok_or_else(|| Error::with_message(Errno::EUCLEAN, "extent block range overflow"))?;
        let blocks_per_group = sb.nr_blocks_per_group();
        let first_data_block = sb.first_data_block();
        let mut current = start;
        while current < end {
            let group_idx = ((current - first_data_block) / blocks_per_group) as usize;
            if group_idx >= sb.nr_block_groups() as usize {
                return_errno_with_message!(
                    Errno::EUCLEAN,
                    "extent block group is outside filesystem"
                );
            }
            let group = fs.block_group(group_idx);
            let group_end = end.min(group.last_block() + 1);
            let range = current..group_end;
            if sb.has_super_block(group_idx) {
                let super_block = sb.bid(group_idx)..sb.bid(group_idx) + 1;
                let descriptors_start = sb.group_descriptors_bid(group_idx);
                let descriptors =
                    descriptors_start..descriptors_start + sb.group_descriptor_blocks_count();
                if ranges_overlap(&range, &super_block) || ranges_overlap(&range, &descriptors) {
                    return_errno_with_message!(
                        Errno::EUCLEAN,
                        "extent block range overlaps superblock metadata"
                    );
                }
            }
            if !group.is_data_area_range(range) {
                return_errno_with_message!(
                    Errno::EUCLEAN,
                    "extent block range overlaps filesystem metadata"
                );
            }
            current = group_end;
        }
        Ok(())
    }
}

fn ranges_overlap<T: Ord>(left: &Range<T>, right: &Range<T>) -> bool {
    left.start < right.end && right.start < left.end
}
