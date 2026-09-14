// SPDX-License-Identifier: MPL-2.0

//! Validated traversal of an EXT4 extent tree.
//!
//! [`ExtentTree`] keeps the inode-resident root in memory. Lookup follows index
//! entries to a leaf. Before returning a mapping, traversal validates the
//! supported tree and rejects duplicate or overlapping block ownership.

use super::node::{Extent, ExtentHeader, ExtentIdx, RawExtent, RawExtentIdx};
use crate::fs::fs_impls::ext4::{fs::Ext4, inode::RAW_BLOCK_PTRS_LEN, prelude::*};

pub(super) const ENTRY_SIZE: usize = 12;

/// An EXT4 extent tree rooted in an inode's 60-byte `i_block` field.
///
/// `depth` always matches the root header. Every parsed node has ordered keys;
/// leaf extents do not overlap logically, and full-tree validation also rejects
/// duplicate or overlapping physical ownership.
#[derive(Debug)]
pub(super) struct ExtentTree {
    root: [u32; RAW_BLOCK_PTRS_LEN],
    depth: u16,
}

impl ExtentTree {
    /// Creates a tree after validating the inode-resident root node.
    pub(super) fn try_from_root(root: [u32; RAW_BLOCK_PTRS_LEN]) -> Result<Self> {
        let bytes = root.as_bytes();
        let header = ExtentHeader::parse(bytes, None)?;
        Self::validate_entries(bytes, header)?;
        Ok(Self {
            root,
            depth: header.depth(),
        })
    }

    /// Finds the extent containing `iblock`.
    ///
    /// `None` means that `iblock` is a hole. A returned extent has a validated
    /// physical range. The current pre-lookup ownership check validates the
    /// complete supported tree before trusting the selected lookup path.
    pub(super) fn find(&self, fs: &Ext4, iblock: Iblock) -> Result<Option<Extent>> {
        // Validate every supported leaf before trusting the lookup path. A point
        // lookup alone cannot detect ownership corruption in a sibling leaf.
        self.flatten(fs)?;
        let mut step = Self::search_node(self.root.as_bytes(), self.depth, iblock)?;
        let mut expected_depth = self.depth;

        while let Step::Descend(block) = step {
            fs.validate_extent_block_range(block, 1)?;
            expected_depth = expected_depth.checked_sub(1).ok_or_else(|| {
                Error::with_message(Errno::EUCLEAN, "extent leaf contains an index")
            })?;
            let bytes = fs
                .block_device()
                .read_val::<[u8; BLOCK_SIZE]>(Bid::new(block.into()).to_offset())?;
            step = Self::search_node(&bytes, expected_depth, iblock)?;
        }

        match step {
            Step::Found(extent) => {
                fs.validate_extent_block_range(extent.start(), u32::from(extent.len()))?;
                Ok(Some(extent))
            }
            Step::Hole => Ok(None),
            Step::Descend(_) => unreachable!(),
        }
    }

    /// Returns the inode-resident extent root.
    pub(super) const fn root(&self) -> [u32; RAW_BLOCK_PTRS_LEN] {
        self.root
    }

    fn flatten(&self, fs: &Ext4) -> Result<(Vec<Extent>, Vec<Ext4Bid>)> {
        let root_header = ExtentHeader::parse(self.root.as_bytes(), Some(self.depth))?;
        if root_header.is_leaf() {
            let mut extents = Vec::with_capacity(root_header.entries());
            for index in 0..root_header.entries() {
                let extent = Self::extent_at(self.root.as_bytes(), index)?;
                fs.validate_extent_block_range(extent.start(), u32::from(extent.len()))?;
                extents.push(extent);
            }
            return Self::validate_ownership(extents, Vec::new());
        }
        if root_header.depth() != 1 {
            return_errno_with_message!(
                Errno::EOPNOTSUPP,
                "extent validation supports depth up to one"
            );
        }

        let mut extents = Vec::new();
        let mut external = Vec::with_capacity(root_header.entries());
        for index in 0..root_header.entries() {
            let entry = Self::index_at(self.root.as_bytes(), index)?;
            let block = entry.leaf();
            fs.validate_extent_block_range(block, 1)?;
            let bytes = fs
                .block_device()
                .read_val::<[u8; BLOCK_SIZE]>(Bid::new(block.into()).to_offset())?;
            let header = ExtentHeader::parse(&bytes, Some(0))?;
            Self::validate_entries(&bytes, header)?;
            for child_index in 0..header.entries() {
                let extent = Self::extent_at(&bytes, child_index)?;
                fs.validate_extent_block_range(extent.start(), u32::from(extent.len()))?;
                extents.push(extent);
            }
            external.push(block);
        }
        Self::validate_ownership(extents, external)
    }

    fn validate_ownership(
        extents: Vec<Extent>,
        external: Vec<Ext4Bid>,
    ) -> Result<(Vec<Extent>, Vec<Ext4Bid>)> {
        let mut logical_extents = extents.clone();
        logical_extents.sort_by_key(|extent| extent.block());
        for pair in logical_extents.windows(2) {
            if pair[0].logical_end() > u64::from(pair[1].block()) {
                return_errno_with_message!(Errno::EUCLEAN, "extent leaves overlap");
            }
        }
        let mut physical_extents = extents.clone();
        physical_extents.sort_by_key(|extent| extent.start());
        for pair in physical_extents.windows(2) {
            if pair[0].start() + u32::from(pair[0].len()) > pair[1].start() {
                return_errno_with_message!(Errno::EUCLEAN, "extent data blocks overlap");
            }
        }
        for (index, &block) in external.iter().enumerate() {
            if external[..index].contains(&block) {
                return_errno_with_message!(Errno::EUCLEAN, "extent leaf block is referenced twice");
            }
            if physical_extents.iter().any(|extent| {
                extent.start() <= block && block < extent.start() + u32::from(extent.len())
            }) {
                return_errno_with_message!(Errno::EUCLEAN, "extent leaf block is also data");
            }
        }
        Ok((extents, external))
    }

    fn search_node(bytes: &[u8], expected_depth: u16, iblock: Iblock) -> Result<Step> {
        let header = ExtentHeader::parse(bytes, Some(expected_depth))?;
        Self::validate_entries(bytes, header)?;

        if header.is_leaf() {
            let mut candidate = None;
            for index in 0..header.entries() {
                let extent = Self::extent_at(bytes, index)?;
                if extent.block() > iblock {
                    break;
                }
                candidate = Some(extent);
            }
            return Ok(match candidate {
                Some(extent) if extent.covers(iblock) => Step::Found(extent),
                _ => Step::Hole,
            });
        }

        let mut candidate = None;
        for index in 0..header.entries() {
            let entry = Self::index_at(bytes, index)?;
            if entry.block() > iblock {
                break;
            }
            candidate = Some(entry);
        }
        Ok(match candidate {
            Some(entry) => Step::Descend(entry.leaf()),
            None => Step::Hole,
        })
    }

    fn validate_entries(bytes: &[u8], header: ExtentHeader) -> Result<()> {
        if header.is_leaf() {
            let mut previous: Option<Extent> = None;
            for index in 0..header.entries() {
                let extent = Self::extent_at(bytes, index)?;
                if previous.is_some_and(|prev| prev.logical_end() > u64::from(extent.block())) {
                    return_errno_with_message!(
                        Errno::EUCLEAN,
                        "extent entries are unsorted or overlapping"
                    );
                }
                previous = Some(extent);
            }
        } else {
            let mut previous = None;
            for index in 0..header.entries() {
                let entry = Self::index_at(bytes, index)?;
                if previous.is_some_and(|block| block >= entry.block()) {
                    return_errno_with_message!(Errno::EUCLEAN, "extent indexes are unsorted");
                }
                previous = Some(entry.block());
            }
        }
        Ok(())
    }

    fn extent_at(bytes: &[u8], index: usize) -> Result<Extent> {
        let offset = ENTRY_SIZE * (index + 1);
        Extent::try_from(&RawExtent::from_bytes(&bytes[offset..offset + ENTRY_SIZE]))
    }

    fn index_at(bytes: &[u8], index: usize) -> Result<ExtentIdx> {
        let offset = ENTRY_SIZE * (index + 1);
        ExtentIdx::try_from(&RawExtentIdx::from_bytes(
            &bytes[offset..offset + ENTRY_SIZE],
        ))
    }
}

enum Step {
    Found(Extent),
    Hole,
    Descend(Ext4Bid),
}
