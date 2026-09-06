// SPDX-License-Identifier: MPL-2.0

//! Read-only traversal of an ext4 extent tree.

use super::node::{Extent, ExtentHeader, ExtentIdx, RawExtent, RawExtentIdx};
use crate::fs::fs_impls::ext4::{inode::RAW_BLOCK_PTRS_LEN, prelude::*};

pub(super) const ENTRY_SIZE: usize = 12;

pub(super) struct ExtentTree {
    root: [u32; RAW_BLOCK_PTRS_LEN],
    depth: u16,
}

impl ExtentTree {
    pub(super) fn try_from_root(root: [u32; RAW_BLOCK_PTRS_LEN]) -> Result<Self> {
        let bytes = root.as_bytes();
        let header = ExtentHeader::parse(bytes, None)?;
        Self::validate_entries(bytes, header)?;
        Ok(Self {
            root,
            depth: header.depth(),
        })
    }

    pub(super) fn find(&self, device: &dyn BlockDevice, iblock: Iblock) -> Result<Option<Extent>> {
        let mut step = Self::search_node(self.root.as_bytes(), self.depth, iblock)?;
        let mut expected_depth = self.depth;

        while let Step::Descend(block) = step {
            expected_depth = expected_depth.checked_sub(1).ok_or_else(|| {
                Error::with_message(Errno::EUCLEAN, "extent leaf contains an index")
            })?;
            let bytes = device.read_val::<[u8; BLOCK_SIZE]>(Bid::new(block.into()).to_offset())?;
            step = Self::search_node(&bytes, expected_depth, iblock)?;
        }

        match step {
            Step::Found(extent) => Ok(Some(extent)),
            Step::Hole => Ok(None),
            Step::Descend(_) => unreachable!(),
        }
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
