// SPDX-License-Identifier: MPL-2.0

//! Per-inode EXT4 extent lookup and validation.
//!
//! [`ExtentState`] stores the format-specific state of an on-disk extent tree.
//! The parent [`BlockMapping`](super::BlockMapping) owns synchronization, the
//! filesystem reference, and page-cache bounds, then delegates extent lookup
//! here.
//!
//! # State and locking
//!
//! The parent mapping lock protects the extent root, sector accounting, and the
//! mapping dirty flag. Read-side lookup holds the corresponding read lock while
//! validating and traversing extent metadata; data BIO submission happens only
//! after that lock has been released.
//!
//! # Invariants
//!
//! - Extents are ordered by logical block and do not overlap logically or
//!   physically.
//! - Extent metadata blocks are valid allocated data-area blocks and are not
//!   also owned by a data extent.
//! - `sector_count` is the inode's on-disk `i_blocks` value in 512-byte sectors.
//! - This commit does not mutate an extent tree. Allocation, truncation, and
//!   metadata writeback are added by the writable extent change.

use super::RawBlockPtrs;
use crate::fs::fs_impls::ext4::{fs::Ext4, inode::RAW_BLOCK_PTRS_LEN, prelude::*};

mod node;
mod tree;

pub(in crate::fs::fs_impls::ext4::inode) fn validate_extent_root(
    root: [u32; RAW_BLOCK_PTRS_LEN],
) -> Result<()> {
    tree::ExtentTree::try_from_root(root).map(|_| ())
}

/// Format-specific state for an inode that uses an EXT4 extent tree.
#[derive(Debug)]
pub(in crate::fs::fs_impls::ext4::inode) struct ExtentState {
    tree: tree::ExtentTree,
    sector_count: u32,
    dirty: bool,
}

impl ExtentState {
    /// Creates an extent mapping from an inode-resident root and sector count.
    pub(super) fn new(root: [u32; RAW_BLOCK_PTRS_LEN], sector_count: u32) -> Result<Self> {
        Ok(Self {
            tree: tree::ExtentTree::try_from_root(root)?,
            sector_count,
            dirty: false,
        })
    }

    /// Resolves one logical block to a physical block.
    pub(super) fn map_block(&self, fs: &Ext4, iblock: Iblock) -> Result<Option<Ext4Bid>> {
        Ok(self
            .tree
            .find(fs, iblock)?
            .map(|extent| extent.physical_block(iblock)))
    }

    /// Returns the mapped physical run beginning at `iblock`, or a hole.
    pub(in crate::fs::fs_impls::ext4::inode) fn mapped_run(
        &self,
        fs: &Ext4,
        iblock: Iblock,
        max_blocks: u32,
    ) -> Result<Option<Range<Ext4Bid>>> {
        let Some(extent) = self.tree.find(fs, iblock)? else {
            return Ok(None);
        };
        let offset = iblock - extent.block();
        let len = (u32::from(extent.len()) - offset).min(max_blocks);
        let start = extent.physical_block(iblock);
        Ok(Some(start..start + len))
    }

    pub(super) fn raw_block_ptrs(&self) -> RawBlockPtrs {
        RawBlockPtrs::new(self.sector_count, self.tree.root())
    }

    pub(super) fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub(super) fn clear_dirty(&mut self) {
        self.dirty = false;
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    use super::{
        node::{EXTENT_MAGIC, RawExtent, RawExtentHeader, RawExtentIdx},
        tree::{ENTRY_SIZE, ExtentTree},
    };
    use crate::fs::fs_impls::ext4::{
        inode::RAW_BLOCK_PTRS_LEN,
        prelude::*,
        test_utils::{BlockBitmapInit, Ext4FixtureBuilder},
    };

    fn inline_root(header: RawExtentHeader, entries: &[RawExtent]) -> [u32; RAW_BLOCK_PTRS_LEN] {
        let mut root = [0u32; RAW_BLOCK_PTRS_LEN];
        let bytes = root.as_mut_bytes();
        bytes[..ENTRY_SIZE].copy_from_slice(header.as_bytes());
        for (index, extent) in entries.iter().enumerate() {
            let offset = ENTRY_SIZE * (index + 1);
            bytes[offset..offset + ENTRY_SIZE].copy_from_slice(extent.as_bytes());
        }
        root
    }

    fn leaf_header(entries: u16) -> RawExtentHeader {
        RawExtentHeader {
            magic: EXTENT_MAGIC,
            entries,
            max: 4,
            depth: 0,
            generation: 0,
        }
    }

    #[ktest]
    fn inline_lookup_distinguishes_mappings_and_holes() {
        let fixture = Ext4FixtureBuilder::new(1, 2048)
            .block_bitmap(BlockBitmapInit::MetadataPlus(vec![40, 41, 42, 45, 46]))
            .build()
            .unwrap();
        let root = inline_root(
            leaf_header(2),
            &[RawExtent::new(2, 3, 40), RawExtent::new(8, 2, 45)],
        );
        let tree = ExtentTree::try_from_root(root).unwrap();

        let extent = tree.find(&fixture.ext2, 3).unwrap().unwrap();
        assert_eq!(extent.physical_block(3), 41);
        assert!(tree.find(&fixture.ext2, 6).unwrap().is_none());
    }

    #[ktest]
    fn block_mapping_routes_extent_lookup() {
        let fixture = Ext4FixtureBuilder::new(1, 2048)
            .block_bitmap(BlockBitmapInit::MetadataPlus(vec![40, 41, 42]))
            .build()
            .unwrap();
        let root = inline_root(leaf_header(1), &[RawExtent::new(0, 3, 40)]);
        let mapping = super::super::BlockMapping::new_extent(
            super::RawBlockPtrs::new(24, root),
            Arc::downgrade(&fixture.ext2),
            4,
        )
        .unwrap();

        assert_eq!(mapping.mapped_run(1, 8).unwrap(), Some(41..43));
        assert_eq!(mapping.mapped_run(3, 8).unwrap(), None);
    }

    #[ktest]
    fn lookup_descends_into_an_external_leaf() {
        let fixture = Ext4FixtureBuilder::new(1, 2048)
            .block_bitmap(BlockBitmapInit::MetadataPlus(vec![40, 45, 46, 47, 48]))
            .build()
            .unwrap();
        let leaf_bid = 40;
        let mut leaf = [0u8; BLOCK_SIZE];
        let header = RawExtentHeader {
            magic: EXTENT_MAGIC,
            entries: 1,
            max: ((BLOCK_SIZE - ENTRY_SIZE) / ENTRY_SIZE) as u16,
            depth: 0,
            generation: 0,
        };
        leaf[..ENTRY_SIZE].copy_from_slice(header.as_bytes());
        leaf[ENTRY_SIZE..2 * ENTRY_SIZE].copy_from_slice(RawExtent::new(4, 4, 45).as_bytes());
        fixture
            .disk
            .segment()
            .write_bytes(Bid::new(leaf_bid.into()).to_offset(), &leaf)
            .unwrap();

        let mut root = [0u32; RAW_BLOCK_PTRS_LEN];
        let bytes = root.as_mut_bytes();
        let root_header = RawExtentHeader {
            magic: EXTENT_MAGIC,
            entries: 1,
            max: 4,
            depth: 1,
            generation: 0,
        };
        bytes[..ENTRY_SIZE].copy_from_slice(root_header.as_bytes());
        bytes[ENTRY_SIZE..2 * ENTRY_SIZE]
            .copy_from_slice(RawExtentIdx::new(0, leaf_bid).as_bytes());

        let tree = ExtentTree::try_from_root(root).unwrap();
        let extent = tree.find(&fixture.ext2, 6).unwrap().unwrap();
        assert_eq!(extent.physical_block(6), 47);
    }

    #[ktest]
    fn rejects_invalid_headers_and_leaf_ordering() {
        let bad_magic = inline_root(
            RawExtentHeader {
                magic: 0,
                ..leaf_header(0)
            },
            &[],
        );
        assert!(ExtentTree::try_from_root(bad_magic).is_err());

        let overlap = inline_root(
            leaf_header(2),
            &[RawExtent::new(2, 4, 100), RawExtent::new(5, 1, 200)],
        );
        assert!(ExtentTree::try_from_root(overlap).is_err());
    }

    #[ktest]
    fn rejects_unsupported_depth_and_48_bit_blocks() {
        let too_deep = inline_root(
            RawExtentHeader {
                depth: 6,
                ..leaf_header(0)
            },
            &[],
        );
        assert!(ExtentTree::try_from_root(too_deep).is_err());

        let high_block = inline_root(
            leaf_header(1),
            &[RawExtent {
                block: 0,
                len: 1,
                start_hi: 1,
                start_lo: 0,
            }],
        );
        assert!(ExtentTree::try_from_root(high_block).is_err());
    }

    #[ktest]
    fn rejects_external_child_with_wrong_depth() {
        let fixture = Ext4FixtureBuilder::new(1, 2048)
            .block_bitmap(BlockBitmapInit::MetadataPlus(vec![40]))
            .build()
            .unwrap();
        let leaf_bid = 40;
        let mut child = [0u8; BLOCK_SIZE];
        let child_header = RawExtentHeader {
            magic: EXTENT_MAGIC,
            entries: 0,
            max: ((BLOCK_SIZE - ENTRY_SIZE) / ENTRY_SIZE) as u16,
            depth: 1,
            generation: 0,
        };
        child[..ENTRY_SIZE].copy_from_slice(child_header.as_bytes());
        fixture
            .disk
            .segment()
            .write_bytes(Bid::new(leaf_bid.into()).to_offset(), &child)
            .unwrap();

        let mut root = [0u32; RAW_BLOCK_PTRS_LEN];
        let bytes = root.as_mut_bytes();
        let root_header = RawExtentHeader {
            magic: EXTENT_MAGIC,
            entries: 1,
            max: 4,
            depth: 1,
            generation: 0,
        };
        bytes[..ENTRY_SIZE].copy_from_slice(root_header.as_bytes());
        bytes[ENTRY_SIZE..2 * ENTRY_SIZE]
            .copy_from_slice(RawExtentIdx::new(0, leaf_bid).as_bytes());

        let tree = ExtentTree::try_from_root(root).unwrap();
        assert!(tree.find(&fixture.ext2, 0).is_err());
    }
}
