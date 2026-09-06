// SPDX-License-Identifier: MPL-2.0

//! Ext4 extent-tree lookup and validation.

mod node;
mod tree;

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    use super::{
        node::{EXTENT_MAGIC, RawExtent, RawExtentHeader, RawExtentIdx},
        tree::{ENTRY_SIZE, ExtentTree},
    };
    use crate::fs::fs_impls::ext4::{
        inode::RAW_BLOCK_PTRS_LEN, prelude::*, test_utils::Ext4FixtureBuilder,
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
        let fixture = Ext4FixtureBuilder::new(1, 2048).build().unwrap();
        let root = inline_root(
            leaf_header(2),
            &[RawExtent::new(2, 3, 100), RawExtent::new(8, 2, 200)],
        );
        let tree = ExtentTree::try_from_root(root).unwrap();
        let device = fixture.ext2.block_device();

        let extent = tree.find(device, 3).unwrap().unwrap();
        assert_eq!(extent.physical_block(3), 101);
        assert!(tree.find(device, 6).unwrap().is_none());
    }

    #[ktest]
    fn lookup_descends_into_an_external_leaf() {
        let fixture = Ext4FixtureBuilder::new(1, 2048).build().unwrap();
        let leaf_bid = 200;
        let mut leaf = [0u8; BLOCK_SIZE];
        let header = RawExtentHeader {
            magic: EXTENT_MAGIC,
            entries: 1,
            max: ((BLOCK_SIZE - ENTRY_SIZE) / ENTRY_SIZE) as u16,
            depth: 0,
            generation: 0,
        };
        leaf[..ENTRY_SIZE].copy_from_slice(header.as_bytes());
        leaf[ENTRY_SIZE..2 * ENTRY_SIZE].copy_from_slice(RawExtent::new(4, 4, 300).as_bytes());
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
        let extent = tree.find(fixture.ext2.block_device(), 6).unwrap().unwrap();
        assert_eq!(extent.physical_block(6), 302);
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
        let fixture = Ext4FixtureBuilder::new(1, 2048).build().unwrap();
        let leaf_bid = 200;
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
        assert!(tree.find(fixture.ext2.block_device(), 0).is_err());
    }
}
