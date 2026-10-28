// SPDX-License-Identifier: MPL-2.0

//! Per-inode EXT4 extent mapping.

#![short_vis_path::add(inode)]

use self::{
    node::{Extent, ExtentNode},
    path::ExtentMappingRange,
};
use super::RawBlockPtrs;
use crate::fs::fs_impls::ext4::{
    fs::Ext4, inode::RAW_BLOCK_PTRS_LEN, prelude::*, utils::ranges_overlap,
};

mod cache;
mod mutation;
mod node;
mod path;
mod status;
pub(super) use mutation::PreparedUpdate;
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
/// Logical ranges use exclusive ends no greater than `Iblock::MAX`;
/// that value is an endpoint, not a mapped logical block.
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
pub(in inode) struct ExtentTree {
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

    /// Looks up the physical block that can be read directly.
    ///
    /// Returns `None` for holes and unwritten extents, which read as zeros.
    pub(super) fn lookup_readable_block(
        &self,
        fs: &Ext4,
        iblock: Iblock,
    ) -> Result<Option<Ext4Bid>> {
        Ok(self
            .find(fs, iblock)?
            .filter(|extent| !extent.is_unwritten())
            .map(|extent| extent.physical_block(iblock)))
    }

    /// Returns the extent root and sector count to store in the raw inode.
    pub(super) fn raw_block_ptrs(&self) -> RawBlockPtrs {
        *self.raw_block_ptrs
    }

    /// Returns whether the inode-resident root or sector count needs writeback.
    pub(super) fn is_inode_dirty(&self) -> bool {
        self.raw_block_ptrs.is_dirty()
    }

    /// Returns whether the root or external extent nodes need writeback.
    pub(super) fn has_dirty_metadata(&self) -> bool {
        self.is_inode_dirty() || self.node_cache.is_dirty()
    }

    /// Clears only the dirty flag for the inode-resident root and sector count.
    ///
    /// External extent nodes are unaffected; `sync_metadata` writes them back
    /// and clears their dirty flags on successful completion.
    pub(super) fn clear_inode_dirty(&mut self) {
        self.raw_block_ptrs.clear_dirty();
    }

    /// Writes dirty external nodes from leaves towards the root.
    pub(super) fn sync_metadata(&self, fs: &Ext4) -> Result<()> {
        self.node_cache.sync(fs)
    }
}

const SECTORS_PER_BLOCK: u32 = (BLOCK_SIZE / SECTOR_SIZE) as u32;

pub(in inode) const fn empty_extent_root() -> [u32; RAW_BLOCK_PTRS_LEN] {
    let magic = node::EXTENT_MAGIC.to_ne_bytes();
    let max = (node::INLINE_NODE_CAPACITY as u16).to_ne_bytes();
    let mut root = [0u32; RAW_BLOCK_PTRS_LEN];
    root[0] = u32::from_ne_bytes([magic[0], magic[1], 0, 0]);
    root[1] = u32::from_ne_bytes([max[0], max[1], 0, 0]);
    root
}

impl ExtentTree {
    /// Validates the inode-resident root's format and entry ordering.
    ///
    /// External nodes and filesystem-relative addresses are checked on lookup.
    pub(in inode) fn validate_root(root: &[u32; RAW_BLOCK_PTRS_LEN]) -> Result<()> {
        ExtentNode::parse(root.as_bytes())?;
        Ok(())
    }

    fn find(&self, fs: &Ext4, iblock: Iblock) -> Result<Option<Extent>> {
        match self.lookup_mapping_range(fs, iblock, Iblock::MAX)? {
            ExtentMappingRange::Mapped { extent, .. } => Ok(Some(extent)),
            ExtentMappingRange::Hole(_) => Ok(None),
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

#[cfg(ktest)]
mod tests;
