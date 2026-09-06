// SPDX-License-Identifier: MPL-2.0

//! Read-only traversal of an ext4 extent tree.

use super::node::{Extent, ExtentHeader, ExtentIdx, RawExtent, RawExtentHeader, RawExtentIdx};
use crate::fs::fs_impls::ext4::{fs::Ext4, inode::RAW_BLOCK_PTRS_LEN, prelude::*};

pub(super) const ENTRY_SIZE: usize = 12;
const INLINE_CAPACITY: usize = (RAW_BLOCK_PTRS_LEN * size_of::<u32>() - ENTRY_SIZE) / ENTRY_SIZE;
const BLOCK_CAPACITY: usize = (BLOCK_SIZE - ENTRY_SIZE) / ENTRY_SIZE;

#[derive(Debug)]
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

    pub(super) const fn root(&self) -> [u32; RAW_BLOCK_PTRS_LEN] {
        self.root
    }

    pub(super) fn insert(
        &mut self,
        fs: &Ext4,
        iblock: Iblock,
        pblock: Ext4Bid,
        len: u16,
    ) -> Result<TreeDelta> {
        let (mut extents, external) = self.flatten(fs.block_device())?;
        let new_extent = Extent::new(iblock, len, pblock);
        if extents.iter().any(|extent| {
            extent.logical_end() > u64::from(iblock)
                && u64::from(extent.block()) < new_extent.logical_end()
        }) {
            return_errno_with_message!(Errno::EEXIST, "extent overlaps an existing mapping");
        }
        extents.push(new_extent);
        Self::merge_extents(&mut extents);
        self.replace(fs, &extents, &external)
    }

    pub(super) fn extents(&self, device: &dyn BlockDevice) -> Result<Vec<Extent>> {
        let (mut extents, _) = self.flatten(device)?;
        extents.sort_by_key(|extent| extent.block());
        Ok(extents)
    }

    pub(super) fn rebuild(&mut self, fs: &Ext4, extents: &[Extent]) -> Result<TreeDelta> {
        let (_, external) = self.flatten(fs.block_device())?;
        self.replace(fs, extents, &external)
    }

    fn flatten(&self, device: &dyn BlockDevice) -> Result<(Vec<Extent>, Vec<Ext4Bid>)> {
        let root_header = ExtentHeader::parse(self.root.as_bytes(), Some(self.depth))?;
        if root_header.is_leaf() {
            let mut extents = Vec::with_capacity(root_header.entries());
            for index in 0..root_header.entries() {
                extents.push(Self::extent_at(self.root.as_bytes(), index)?);
            }
            return Ok((extents, Vec::new()));
        }
        if root_header.depth() != 1 {
            return_errno_with_message!(Errno::EOPNOTSUPP, "writable extent depth exceeds one");
        }

        let mut extents = Vec::new();
        let mut external = Vec::with_capacity(root_header.entries());
        for index in 0..root_header.entries() {
            let entry = Self::index_at(self.root.as_bytes(), index)?;
            let block = entry.leaf();
            let bytes = device.read_val::<[u8; BLOCK_SIZE]>(Bid::new(block.into()).to_offset())?;
            let header = ExtentHeader::parse(&bytes, Some(0))?;
            Self::validate_entries(&bytes, header)?;
            for child_index in 0..header.entries() {
                extents.push(Self::extent_at(&bytes, child_index)?);
            }
            external.push(block);
        }
        for pair in extents.windows(2) {
            if pair[0].logical_end() > u64::from(pair[1].block()) {
                return_errno_with_message!(Errno::EUCLEAN, "extent leaves overlap");
            }
        }
        Ok((extents, external))
    }

    fn merge_extents(extents: &mut Vec<Extent>) {
        extents.sort_by_key(|extent| extent.block());
        let mut merged: Vec<Extent> = Vec::with_capacity(extents.len());
        for extent in extents.drain(..) {
            if let Some(previous) = merged.last_mut()
                && previous.logical_end() == u64::from(extent.block())
                && u64::from(previous.start()) + u64::from(previous.len())
                    == u64::from(extent.start())
                && u32::from(previous.len()) + u32::from(extent.len()) <= 32768
            {
                *previous = Extent::new(
                    previous.block(),
                    previous.len() + extent.len(),
                    previous.start(),
                );
                continue;
            }
            merged.push(extent);
        }
        *extents = merged;
    }

    fn replace(
        &mut self,
        fs: &Ext4,
        extents: &[Extent],
        old_external: &[Ext4Bid],
    ) -> Result<TreeDelta> {
        if extents.len() <= INLINE_CAPACITY {
            self.root = Self::leaf_root(extents);
            self.depth = 0;
            for &block in old_external {
                fs.free_blocks(block, 1)?;
            }
            return Ok(TreeDelta {
                allocated: 0,
                freed: old_external.len() as u32,
            });
        }

        let leaf_count = extents.len().div_ceil(BLOCK_CAPACITY);
        if leaf_count > INLINE_CAPACITY {
            return_errno_with_message!(Errno::ENOSPC, "extent tree exceeds writable depth");
        }

        let mut new_blocks = Vec::with_capacity(leaf_count);
        let goal = extents.first().map(|extent| extent.start()).unwrap_or(0);
        for _ in 0..leaf_count {
            match fs.alloc_blocks(1, goal) {
                Ok(range) => new_blocks.push(range.start),
                Err(error) => {
                    for &block in &new_blocks {
                        let _ = fs.free_blocks(block, 1);
                    }
                    return Err(error);
                }
            }
        }

        for (chunk, &block) in extents.chunks(BLOCK_CAPACITY).zip(&new_blocks) {
            let bytes = Self::leaf_block(chunk);
            if let Err(error) = fs
                .block_device()
                .write_val(Bid::new(block.into()).to_offset(), &bytes)
            {
                for &allocated in &new_blocks {
                    let _ = fs.free_blocks(allocated, 1);
                }
                return Err(error.into());
            }
        }

        let mut root = [0u32; RAW_BLOCK_PTRS_LEN];
        let bytes = root.as_mut_bytes();
        let header = RawExtentHeader {
            magic: super::node::EXTENT_MAGIC,
            entries: leaf_count as u16,
            max: INLINE_CAPACITY as u16,
            depth: 1,
            generation: 0,
        };
        bytes[..ENTRY_SIZE].copy_from_slice(header.as_bytes());
        for (index, (chunk, &block)) in extents.chunks(BLOCK_CAPACITY).zip(&new_blocks).enumerate()
        {
            let offset = ENTRY_SIZE * (index + 1);
            bytes[offset..offset + ENTRY_SIZE]
                .copy_from_slice(RawExtentIdx::new(chunk[0].block(), block).as_bytes());
        }
        self.root = root;
        self.depth = 1;
        for &block in old_external {
            fs.free_blocks(block, 1)?;
        }
        Ok(TreeDelta {
            allocated: new_blocks.len() as u32,
            freed: old_external.len() as u32,
        })
    }

    fn leaf_root(extents: &[Extent]) -> [u32; RAW_BLOCK_PTRS_LEN] {
        let mut root = [0u32; RAW_BLOCK_PTRS_LEN];
        let bytes = root.as_mut_bytes();
        Self::write_leaf(bytes, INLINE_CAPACITY, extents);
        root
    }

    fn leaf_block(extents: &[Extent]) -> [u8; BLOCK_SIZE] {
        let mut block = [0u8; BLOCK_SIZE];
        Self::write_leaf(&mut block, BLOCK_CAPACITY, extents);
        block
    }

    fn write_leaf(bytes: &mut [u8], capacity: usize, extents: &[Extent]) {
        let header = RawExtentHeader {
            magic: super::node::EXTENT_MAGIC,
            entries: extents.len() as u16,
            max: capacity as u16,
            depth: 0,
            generation: 0,
        };
        bytes[..ENTRY_SIZE].copy_from_slice(header.as_bytes());
        for (index, extent) in extents.iter().enumerate() {
            let offset = ENTRY_SIZE * (index + 1);
            bytes[offset..offset + ENTRY_SIZE].copy_from_slice(extent.to_raw().as_bytes());
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

pub(super) struct TreeDelta {
    pub(super) allocated: u32,
    pub(super) freed: u32,
}
