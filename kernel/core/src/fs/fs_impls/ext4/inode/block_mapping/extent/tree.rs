// SPDX-License-Identifier: MPL-2.0

use super::{
    ExtentTree,
    node::{Extent, ExtentHeader, ExtentIdx, RawExtent, RawExtentHeader, RawExtentIdx},
};
use crate::fs::fs_impls::ext4::{fs::Ext4, inode::RAW_BLOCK_PTRS_LEN, prelude::*};

pub(super) const ENTRY_SIZE: usize = 12;
const INLINE_CAPACITY: usize = (RAW_BLOCK_PTRS_LEN * size_of::<u32>() - ENTRY_SIZE) / ENTRY_SIZE;
const BLOCK_CAPACITY: usize = (BLOCK_SIZE - ENTRY_SIZE) / ENTRY_SIZE;

impl ExtentTree {
    /// Validates the inode-resident root before constructing a tree.
    pub(super) fn validate_root(root: &[u32; RAW_BLOCK_PTRS_LEN]) -> Result<()> {
        let bytes = root.as_bytes();
        let header = ExtentHeader::parse(bytes, None)?;
        Self::validate_entries(bytes, header)
    }

    /// Finds the extent containing `iblock`.
    ///
    /// `None` means that `iblock` is a hole. A returned extent has a validated
    /// physical range. The current pre-lookup ownership check flattens the tree
    /// and therefore limits this operation to trees supported by the writer.
    pub(super) fn find(&self, fs: &Ext4, iblock: Iblock) -> Result<Option<Extent>> {
        // Validate every supported leaf before trusting the lookup path. A point
        // lookup alone cannot detect ownership corruption in a sibling leaf.
        self.flatten(fs)?;
        let depth = ExtentHeader::parse(self.raw_block_ptrs.block_ptrs.as_bytes(), None)?.depth();
        let mut step = Self::search_node(self.raw_block_ptrs.block_ptrs.as_bytes(), depth, iblock)?;
        let mut expected_depth = depth;

        while let Step::Descend(block) = step {
            Self::validate_extent_block_range(fs, block, 1)?;
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
                Self::validate_extent_block_range(fs, extent.start(), u32::from(extent.len()))?;
                Ok(Some(extent))
            }
            Step::Hole => Ok(None),
            Step::Descend(_) => unreachable!(),
        }
    }

    /// Returns all extents in logical-block order after full-tree validation.
    pub(super) fn extents(&self, fs: &Ext4) -> Result<Vec<Extent>> {
        let (mut extents, _) = self.flatten(fs)?;
        extents.sort_by_key(|extent| extent.block());
        Ok(extents)
    }

    /// Builds a replacement root and reports metadata block changes.
    ///
    /// `extents` must be in logical-block order and obey the non-overlap
    /// invariants. The caller publishes the returned root with updated sector
    /// accounting. This operation writes new leaves and releases old leaves;
    /// an error does not roll back those device-side effects.
    pub(super) fn rebuild(
        &self,
        fs: &Ext4,
        extents: &[Extent],
    ) -> Result<([u32; RAW_BLOCK_PTRS_LEN], TreeDelta)> {
        let (_, external) = self.flatten(fs)?;
        self.replace(fs, extents, &external)
    }

    /// Predicts the external metadata block changes for a rebuild.
    ///
    /// Callers use this to validate `i_blocks` arithmetic before mutating the
    /// tree. The result assumes the subsequent rebuild receives the same extent
    /// sequence while the caller retains exclusive access to the tree.
    pub(super) fn anticipated_rebuild_delta(&self, extents: &[Extent]) -> Result<TreeDelta> {
        let header = ExtentHeader::parse(self.raw_block_ptrs.block_ptrs.as_bytes(), None)?;
        if header.depth() > 1 {
            return_errno_with_message!(Errno::EOPNOTSUPP, "writable extent depth exceeds one");
        }
        let allocated = Self::required_external_blocks(extents.len())?;
        let freed = if header.depth() == 0 {
            0
        } else {
            header.entries() as u32
        };
        Ok(TreeDelta { allocated, freed })
    }

    fn required_external_blocks(extent_count: usize) -> Result<u32> {
        if extent_count <= INLINE_CAPACITY {
            return Ok(0);
        }
        let leaf_count = extent_count.div_ceil(BLOCK_CAPACITY);
        if leaf_count > INLINE_CAPACITY {
            return_errno_with_message!(Errno::EFBIG, "extent tree exceeds writable depth");
        }
        Ok(leaf_count as u32)
    }

    fn flatten(&self, fs: &Ext4) -> Result<(Vec<Extent>, Vec<Ext4Bid>)> {
        let root_header = ExtentHeader::parse(self.raw_block_ptrs.block_ptrs.as_bytes(), None)?;
        if root_header.is_leaf() {
            let mut extents = Vec::with_capacity(root_header.entries());
            for index in 0..root_header.entries() {
                let extent = Self::extent_at(self.raw_block_ptrs.block_ptrs.as_bytes(), index)?;
                Self::validate_extent_block_range(fs, extent.start(), u32::from(extent.len()))?;
                extents.push(extent);
            }
            return Self::validate_ownership(extents, Vec::new());
        }
        if root_header.depth() != 1 {
            return_errno_with_message!(Errno::EOPNOTSUPP, "writable extent depth exceeds one");
        }

        let mut extents = Vec::new();
        let mut external = Vec::with_capacity(root_header.entries());
        for index in 0..root_header.entries() {
            let entry = Self::index_at(self.raw_block_ptrs.block_ptrs.as_bytes(), index)?;
            let block = entry.leaf();
            Self::validate_extent_block_range(fs, block, 1)?;
            let bytes = fs
                .block_device()
                .read_val::<[u8; BLOCK_SIZE]>(Bid::new(block.into()).to_offset())?;
            let header = ExtentHeader::parse(&bytes, Some(0))?;
            Self::validate_entries(&bytes, header)?;
            for child_index in 0..header.entries() {
                let extent = Self::extent_at(&bytes, child_index)?;
                Self::validate_extent_block_range(fs, extent.start(), u32::from(extent.len()))?;
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

    /// Writes replacement leaves and returns the candidate inode-resident root.
    ///
    /// External leaves are written directly because there is no extent metadata
    /// cache; inline roots need no metadata I/O. These writes issue no device
    /// flush. Old external blocks are released before the candidate root is returned.
    fn replace(
        &self,
        fs: &Ext4,
        extents: &[Extent],
        old_external: &[Ext4Bid],
    ) -> Result<([u32; RAW_BLOCK_PTRS_LEN], TreeDelta)> {
        if extents.len() <= INLINE_CAPACITY {
            let root = Self::leaf_root(extents);
            let freed = free_old_external_blocks(fs, old_external)?;
            return Ok((
                root,
                TreeDelta {
                    allocated: 0,
                    freed,
                },
            ));
        }

        let leaf_count = Self::required_external_blocks(extents.len())? as usize;

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
        let freed = free_old_external_blocks(fs, old_external)?;
        Ok((
            root,
            TreeDelta {
                allocated: new_blocks.len() as u32,
                freed,
            },
        ))
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
            if !group.is_allocated_data_range(range) {
                return_errno_with_message!(
                    Errno::EUCLEAN,
                    "extent block range is free or overlaps filesystem metadata"
                );
            }
            current = group_end;
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

/// The external extent-tree blocks allocated and freed by a replacement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TreeDelta {
    pub(super) allocated: u32,
    pub(super) freed: u32,
}

fn free_old_external_blocks(fs: &Ext4, blocks: &[Ext4Bid]) -> Result<u32> {
    let mut freed = 0;
    for &block in blocks {
        fs.free_blocks(block, 1)?;
        freed += 1;
    }
    Ok(freed)
}

fn ranges_overlap<T: Ord>(left: &Range<T>, right: &Range<T>) -> bool {
    left.start < right.end && right.start < left.end
}
