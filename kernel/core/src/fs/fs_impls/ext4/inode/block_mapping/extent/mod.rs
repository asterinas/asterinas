// SPDX-License-Identifier: MPL-2.0

//! Ext4 extent-tree lookup and validation.

use core::sync::atomic::{AtomicUsize, Ordering};

use aster_block::bio::BioCompleteFn;
use ostd::mm::io::util::HasVmReaderWriter;

use super::super::block_manager::RawBlockPtrs;
use crate::fs::fs_impls::ext4::{fs::Ext4, inode::RAW_BLOCK_PTRS_LEN, prelude::*};

mod node;
mod tree;

const SECTORS_PER_BLOCK: u32 = (BLOCK_SIZE / SECTOR_SIZE) as u32;
const ZERO_BATCH_BLOCKS: usize = 256;

pub(in crate::fs::fs_impls::ext4::inode) fn empty_extent_root() -> [u32; RAW_BLOCK_PTRS_LEN] {
    let header = node::RawExtentHeader {
        magic: node::EXTENT_MAGIC,
        entries: 0,
        max: 4,
        depth: 0,
        generation: 0,
    };
    let mut root = [0u32; RAW_BLOCK_PTRS_LEN];
    root.as_mut_bytes()[..tree::ENTRY_SIZE].copy_from_slice(header.as_bytes());
    root
}

pub(in crate::fs::fs_impls::ext4::inode) fn validate_extent_root(
    root: [u32; RAW_BLOCK_PTRS_LEN],
) -> Result<()> {
    tree::ExtentTree::try_from_root(root).map(|_| ())
}

#[derive(Debug)]
struct ExtentState {
    tree: tree::ExtentTree,
    sector_count: u32,
    dirty: bool,
}

#[derive(Debug)]
pub(in crate::fs::fs_impls::ext4::inode) struct ExtentManager {
    state: RwMutex<ExtentState>,
    npages: AtomicUsize,
    fs: Weak<Ext4>,
}

impl ExtentManager {
    pub(super) fn new(
        root: [u32; RAW_BLOCK_PTRS_LEN],
        sector_count: u32,
        fs: Weak<Ext4>,
        npages: usize,
    ) -> Result<Self> {
        Ok(Self {
            state: RwMutex::new(ExtentState {
                tree: tree::ExtentTree::try_from_root(root)?,
                sector_count,
                dirty: false,
            }),
            npages: AtomicUsize::new(npages),
            fs,
        })
    }

    fn fs(&self) -> Result<Arc<Ext4>> {
        self.fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "filesystem already dropped"))
    }

    pub(super) fn map_block(&self, iblock: Iblock) -> Result<Option<Ext4Bid>> {
        let fs = self.fs()?;
        Ok(self
            .state
            .read()
            .tree
            .find(&fs, iblock)?
            .map(|extent| extent.physical_block(iblock)))
    }

    pub(in crate::fs::fs_impls::ext4::inode) fn mapped_run(
        &self,
        iblock: Iblock,
        max_blocks: u32,
    ) -> Result<Option<Range<Ext4Bid>>> {
        let fs = self.fs()?;
        let state = self.state.read();
        let Some(extent) = state.tree.find(&fs, iblock)? else {
            return Ok(None);
        };
        let offset = iblock - extent.block();
        let len = (u32::from(extent.len()) - offset).min(max_blocks);
        let start = extent.physical_block(iblock);
        Ok(Some(start..start + len))
    }

    pub(super) fn raw_block_ptrs(&self) -> RawBlockPtrs {
        let state = self.state.read();
        RawBlockPtrs::new(state.sector_count, state.tree.root())
    }

    pub(super) fn is_dirty(&self) -> bool {
        self.state.read().dirty
    }

    pub(super) fn clear_dirty(&self) {
        self.state.write().dirty = false;
    }

    pub(super) fn set_npages(&self, npages: usize) {
        self.npages.store(npages, Ordering::Release);
    }

    pub(super) fn allocate_range_blocks(&self, start: usize, end: usize) -> Result<()> {
        let start = Iblock::try_from(start)
            .map_err(|_| Error::with_message(Errno::EINVAL, "logical block number overflow"))?;
        let end = Iblock::try_from(end)
            .map_err(|_| Error::with_message(Errno::EINVAL, "logical block number overflow"))?;
        if start >= end {
            return Ok(());
        }

        let fs = self.fs()?;
        let mut state = self.state.write();
        let existing = state.tree.extents(&fs)?;
        let mut allocated = Vec::new();
        let mut additions = Vec::new();
        for hole in holes(&existing, start, end) {
            let mut iblock = hole.start;
            while iblock < hole.end {
                let requested = (hole.end - iblock).min(32768);
                let range = match fs.alloc_blocks(requested, 0) {
                    Ok(range) => range,
                    Err(error) => {
                        free_ranges(&fs, &allocated);
                        return Err(error);
                    }
                };
                let len = range.len() as u32;
                if let Err(error) = zero_new_blocks(&fs, &range) {
                    let _ = fs.free_blocks(range.start, len);
                    free_ranges(&fs, &allocated);
                    return Err(error);
                }
                let extent_len = match u16::try_from(len) {
                    Ok(len) if len <= 32768 => len,
                    _ => {
                        let _ = fs.free_blocks(range.start, len);
                        free_ranges(&fs, &allocated);
                        return_errno_with_message!(
                            Errno::EOVERFLOW,
                            "allocated extent is too long"
                        );
                    }
                };
                additions.push(node::Extent::new(iblock, extent_len, range.start));
                allocated.push(range);
                iblock += len;
            }
        }

        if additions.is_empty() {
            return Ok(());
        }
        let mut replacement = existing;
        replacement.extend_from_slice(&additions);
        replacement.sort_by_key(|extent| extent.block());
        replacement = merge_adjacent_extents(replacement);
        let anticipated = match state.tree.anticipated_rebuild_delta(&replacement) {
            Ok(delta) => delta,
            Err(error) => {
                free_ranges(&fs, &allocated);
                return Err(error);
            }
        };
        let data_blocks = allocated.iter().map(|range| range.len() as u32).sum();
        let mut sector_count = state.sector_count;
        if let Err(error) = add_sectors(
            &mut sector_count,
            data_blocks,
            tree::TreeDelta {
                allocated: anticipated.allocated,
                freed: 0,
            },
        ) {
            free_ranges(&fs, &allocated);
            return Err(error);
        }
        let delta = match state.tree.rebuild(&fs, &replacement) {
            Ok(delta) => delta,
            Err(error) => {
                free_ranges(&fs, &allocated);
                return Err(error);
            }
        };
        sector_count = state.sector_count;
        add_sectors(&mut sector_count, data_blocks, delta)?;
        state.sector_count = sector_count;
        state.dirty = true;
        Ok(())
    }

    pub(super) fn truncate_to_byte_len(&self, new_size: usize) -> Result<()> {
        let fs = self.fs()?;
        let keep_blocks = Iblock::try_from(new_size.div_ceil(BLOCK_SIZE))
            .map_err(|_| Error::with_message(Errno::EINVAL, "logical block number overflow"))?;
        let mut state = self.state.write();
        let extents = state.tree.extents(&fs)?;
        let mut kept = Vec::new();
        let mut freed = Vec::new();
        for extent in extents {
            if extent.logical_end() <= u64::from(keep_blocks) {
                kept.push(extent);
            } else if extent.block() >= keep_blocks {
                freed.push(extent.start()..extent.start() + u32::from(extent.len()));
            } else {
                let keep_len = (keep_blocks - extent.block()) as u16;
                kept.push(node::Extent::new(extent.block(), keep_len, extent.start()));
                let free_start = extent.start() + u32::from(keep_len);
                freed.push(free_start..extent.start() + u32::from(extent.len()));
            }
        }
        if freed.is_empty() {
            return Ok(());
        }

        let anticipated = state.tree.anticipated_rebuild_delta(&kept)?;
        let planned_data_blocks = freed.iter().map(|range| range.len() as u32).sum();
        let mut sector_count = state.sector_count;
        subtract_sectors(&mut sector_count, planned_data_blocks, anticipated)?;

        let delta = state.tree.rebuild(&fs, &kept)?;
        let mut data_blocks = 0;
        let mut release_error = None;
        for range in &freed {
            match fs.free_blocks(range.start, range.len() as u32) {
                Ok(()) => data_blocks += range.len() as u32,
                Err(error) => {
                    release_error.get_or_insert(error);
                }
            }
        }
        sector_count = state.sector_count;
        subtract_sectors(&mut sector_count, data_blocks, delta)?;
        state.sector_count = sector_count;
        state.dirty = true;
        if let Some(error) = release_error {
            return Err(error);
        }
        Ok(())
    }
}

fn holes(extents: &[node::Extent], start: Iblock, end: Iblock) -> Vec<Range<Iblock>> {
    let mut result = Vec::new();
    let mut cursor = start;
    for extent in extents {
        if extent.logical_end() <= u64::from(cursor) {
            continue;
        }
        if extent.block() >= end {
            break;
        }
        if extent.block() > cursor {
            result.push(cursor..extent.block().min(end));
        }
        cursor = cursor.max(extent.logical_end() as Iblock);
        if cursor >= end {
            break;
        }
    }
    if cursor < end {
        result.push(cursor..end);
    }
    result
}

fn merge_adjacent_extents(extents: Vec<node::Extent>) -> Vec<node::Extent> {
    let mut merged: Vec<node::Extent> = Vec::with_capacity(extents.len());
    for extent in extents {
        let Some(previous) = merged.last_mut() else {
            merged.push(extent);
            continue;
        };
        let combined_len = u32::from(previous.len()) + u32::from(extent.len());
        if previous.logical_end() == u64::from(extent.block())
            && previous.start() + u32::from(previous.len()) == extent.start()
            && combined_len <= 32768
        {
            *previous = node::Extent::new(previous.block(), combined_len as u16, previous.start());
        } else {
            merged.push(extent);
        }
    }
    merged
}

fn free_ranges(fs: &Ext4, ranges: &[Range<Ext4Bid>]) {
    for range in ranges {
        let _ = fs.free_blocks(range.start, range.len() as u32);
    }
}

fn zero_new_blocks(fs: &Ext4, block_range: &Range<Ext4Bid>) -> Result<()> {
    let mut start = block_range.start;
    while start < block_range.end {
        let blocks = usize::try_from(block_range.end - start)
            .unwrap()
            .min(ZERO_BATCH_BLOCKS);
        let mut io_batch = IoBatch::with_capacity(1);
        let segment = BioSegment::alloc(blocks, BioDirection::ToDevice);
        segment.writer().unwrap().fill_zeros(blocks * BLOCK_SIZE);
        fs.write_blocks_async(start, segment, None, &mut io_batch)?;
        io_batch.wait_all()?;
        start += blocks as u32;
    }
    Ok(())
}

fn add_sectors(count: &mut u32, data_blocks: u32, delta: tree::TreeDelta) -> Result<()> {
    let blocks = data_blocks as i64 + i64::from(delta.allocated) - i64::from(delta.freed);
    let sectors = blocks * i64::from(SECTORS_PER_BLOCK);
    *count = u32::try_from(i64::from(*count) + sectors)
        .map_err(|_| Error::with_message(Errno::EOVERFLOW, "i_blocks accounting overflow"))?;
    Ok(())
}

fn subtract_sectors(count: &mut u32, data_blocks: u32, delta: tree::TreeDelta) -> Result<()> {
    let blocks = data_blocks as i64 + i64::from(delta.freed) - i64::from(delta.allocated);
    let sectors = blocks * i64::from(SECTORS_PER_BLOCK);
    *count = u32::try_from(i64::from(*count) - sectors)
        .map_err(|_| Error::with_message(Errno::EUCLEAN, "invalid i_blocks accounting"))?;
    Ok(())
}

impl BlockAsPageCacheBackend for ExtentManager {
    fn submit_read_bio(
        &self,
        idx: usize,
        bio_segment: BioSegment,
        complete_fn: BioCompleteFn,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        if idx >= self.npages.load(Ordering::Acquire) {
            return_errno_with_message!(Errno::EINVAL, "invalid read size");
        }
        let iblock = Iblock::try_from(idx)
            .map_err(|_| Error::with_message(Errno::EINVAL, "logical block number overflow"))?;
        match self.map_block(iblock)? {
            Some(block) => {
                self.fs()?
                    .read_blocks_async(block, bio_segment, Some(complete_fn), io_batch)
            }
            None => {
                complete_fn(BioStatus::Zeros);
                Ok(())
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
        if idx >= self.npages.load(Ordering::Acquire) {
            return_errno_with_message!(Errno::EINVAL, "invalid write size");
        }
        let iblock = Iblock::try_from(idx)
            .map_err(|_| Error::with_message(Errno::EINVAL, "logical block number overflow"))?;
        if self.map_block(iblock)?.is_none() {
            self.allocate_range_blocks(idx, idx + bio_segment.nblocks())?;
        }
        let block = self
            .map_block(iblock)?
            .ok_or_else(|| Error::with_message(Errno::EIO, "extent allocation produced a hole"))?;
        self.fs()?
            .write_blocks_async(block, bio_segment, Some(complete_fn), io_batch)
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    use super::{
        ExtentManager, ZERO_BATCH_BLOCKS,
        node::{EXTENT_MAGIC, Extent, RawExtent, RawExtentHeader, RawExtentIdx},
        tree::{ENTRY_SIZE, ExtentTree},
    };
    use crate::fs::fs_impls::ext4::{
        inode::RAW_BLOCK_PTRS_LEN,
        prelude::*,
        test_utils::{BlockBitmapInit, Ext4FixtureBuilder, assert_errno, group0_layout},
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

    #[ktest]
    fn inline_overflow_grows_a_depth_one_tree() {
        let fixture = Ext4FixtureBuilder::new(1, 2048)
            .with_free_blocks(64, 64)
            .block_bitmap(BlockBitmapInit::MetadataPlus(vec![40, 42, 44, 46, 48]))
            .build()
            .unwrap();
        let mut tree = ExtentTree::try_from_root(inline_root(leaf_header(0), &[])).unwrap();

        let extents: Vec<_> = (0..5)
            .map(|index| Extent::new(index * 2, 1, 40 + index * 2))
            .collect();
        tree.rebuild(&fixture.ext2, &extents).unwrap();

        for index in 0..5 {
            let extent = tree.find(&fixture.ext2, index * 2).unwrap().unwrap();
            assert_eq!(extent.physical_block(index * 2), 40 + index * 2);
        }
    }

    #[ktest]
    fn allocation_and_truncate_restore_free_blocks() {
        let fixture = Ext4FixtureBuilder::new(1, 2048)
            .with_free_blocks(64, 64)
            .build()
            .unwrap();
        let manager = ExtentManager::new(
            inline_root(leaf_header(0), &[]),
            0,
            Arc::downgrade(&fixture.ext2),
            8,
        )
        .unwrap();
        let free_before = fixture.ext2.super_block().free_blocks_count();

        manager.allocate_range_blocks(0, 8).unwrap();
        assert!(fixture.ext2.super_block().free_blocks_count() < free_before);
        manager.truncate_to_byte_len(0).unwrap();
        assert_eq!(fixture.ext2.super_block().free_blocks_count(), free_before);
    }

    #[ktest]
    fn repeated_contiguous_allocations_merge_into_one_extent() {
        let fixture = Ext4FixtureBuilder::new(1, 2048)
            .with_free_blocks(64, 64)
            .build()
            .unwrap();
        let manager = ExtentManager::new(
            inline_root(leaf_header(0), &[]),
            0,
            Arc::downgrade(&fixture.ext2),
            8,
        )
        .unwrap();

        for block in 0..8 {
            manager.allocate_range_blocks(block, block + 1).unwrap();
        }

        let state = manager.state.read();
        let extents = state.tree.extents(&fixture.ext2).unwrap();
        assert_eq!(extents.len(), 1);
        assert_eq!(extents[0].len(), 8);
    }

    #[ktest]
    fn allocation_zeroes_data_blocks_before_mapping_them() {
        let fixture = Ext4FixtureBuilder::new(1, 2048)
            .with_free_blocks(64, 64)
            .build()
            .unwrap();
        let manager = ExtentManager::new(
            inline_root(leaf_header(0), &[]),
            0,
            Arc::downgrade(&fixture.ext2),
            1,
        )
        .unwrap();
        let expected_bid = group0_layout(&fixture.sb).first_data_bid;
        let stale = [0xa5; BLOCK_SIZE];
        fixture
            .disk
            .segment()
            .write_bytes(Bid::new(expected_bid.into()).to_offset(), &stale)
            .unwrap();

        manager.allocate_range_blocks(0, 1).unwrap();
        let mapped = manager.map_block(0).unwrap().unwrap();
        assert_eq!(mapped, expected_bid);
        let mut contents = [0xff; BLOCK_SIZE];
        fixture
            .disk
            .segment()
            .read_bytes(Bid::new(mapped.into()).to_offset(), &mut contents)
            .unwrap();
        assert_eq!(contents, [0; BLOCK_SIZE]);
    }

    #[ktest]
    fn allocation_zeroes_large_ranges_in_bounded_write_batches() {
        let fixture = Ext4FixtureBuilder::new(1, 512)
            .with_blocks_per_group(512)
            .with_free_blocks(300, 300)
            .build()
            .unwrap();
        let manager = ExtentManager::new(
            inline_root(leaf_header(0), &[]),
            0,
            Arc::downgrade(&fixture.ext2),
            257,
        )
        .unwrap();

        manager.allocate_range_blocks(0, 257).unwrap();

        assert!(fixture.disk.max_write_blocks() <= ZERO_BATCH_BLOCKS);
    }

    #[ktest]
    fn rejects_extent_blocks_outside_the_filesystem() {
        let fixture = Ext4FixtureBuilder::new(1, 2048).build().unwrap();
        let outside = fixture.sb.total_blocks();
        let root = inline_root(leaf_header(1), &[RawExtent::new(0, 1, outside)]);
        let manager = ExtentManager::new(root, 8, Arc::downgrade(&fixture.ext2), 1).unwrap();

        assert!(manager.map_block(0).is_err());
    }

    #[ktest]
    fn rejects_extent_blocks_in_filesystem_metadata() {
        let fixture = Ext4FixtureBuilder::new(1, 2048).build().unwrap();
        let metadata_bid = group0_layout(&fixture.sb).block_bitmap_bid;
        let root = inline_root(leaf_header(1), &[RawExtent::new(0, 1, metadata_bid)]);
        let manager = ExtentManager::new(root, 8, Arc::downgrade(&fixture.ext2), 1).unwrap();

        assert!(manager.map_block(0).is_err());
    }

    #[ktest]
    fn rejects_inline_extent_in_primary_group_descriptor_table() {
        let fixture = Ext4FixtureBuilder::new(1, 2048)
            .block_bitmap(BlockBitmapInit::MetadataOnly)
            .build()
            .unwrap();
        let descriptor_bid = fixture.sb.group_descriptors_bid(0);
        let root = inline_root(leaf_header(1), &[RawExtent::new(0, 1, descriptor_bid)]);
        let tree = ExtentTree::try_from_root(root).unwrap();

        assert_errno!(tree.find(&fixture.ext2, 0), Errno::EUCLEAN);
    }

    #[ktest]
    fn rejects_extent_index_in_primary_group_descriptor_table() {
        let fixture = Ext4FixtureBuilder::new(1, 2048)
            .block_bitmap(BlockBitmapInit::MetadataPlus(vec![40]))
            .build()
            .unwrap();
        let descriptor_bid = fixture.sb.group_descriptors_bid(0);
        let mut leaf = [0u8; BLOCK_SIZE];
        leaf[..ENTRY_SIZE].copy_from_slice(
            RawExtentHeader {
                magic: EXTENT_MAGIC,
                entries: 1,
                max: ((BLOCK_SIZE - ENTRY_SIZE) / ENTRY_SIZE) as u16,
                depth: 0,
                generation: 0,
            }
            .as_bytes(),
        );
        leaf[ENTRY_SIZE..2 * ENTRY_SIZE].copy_from_slice(RawExtent::new(0, 1, 40).as_bytes());
        fixture
            .disk
            .segment()
            .write_bytes(Bid::new(descriptor_bid.into()).to_offset(), &leaf)
            .unwrap();

        let mut root = [0u32; RAW_BLOCK_PTRS_LEN];
        let bytes = root.as_mut_bytes();
        bytes[..ENTRY_SIZE].copy_from_slice(
            RawExtentHeader {
                magic: EXTENT_MAGIC,
                entries: 1,
                max: 4,
                depth: 1,
                generation: 0,
            }
            .as_bytes(),
        );
        bytes[ENTRY_SIZE..2 * ENTRY_SIZE]
            .copy_from_slice(RawExtentIdx::new(0, descriptor_bid).as_bytes());
        let tree = ExtentTree::try_from_root(root).unwrap();

        assert_errno!(tree.find(&fixture.ext2, 0), Errno::EUCLEAN);
    }

    #[ktest]
    fn rejects_extent_index_blocks_outside_the_filesystem() {
        let fixture = Ext4FixtureBuilder::new(1, 2048).build().unwrap();
        let mut root = [0u32; RAW_BLOCK_PTRS_LEN];
        let bytes = root.as_mut_bytes();
        let header = RawExtentHeader {
            magic: EXTENT_MAGIC,
            entries: 1,
            max: 4,
            depth: 1,
            generation: 0,
        };
        bytes[..ENTRY_SIZE].copy_from_slice(header.as_bytes());
        bytes[ENTRY_SIZE..2 * ENTRY_SIZE]
            .copy_from_slice(RawExtentIdx::new(0, fixture.sb.total_blocks()).as_bytes());
        let manager = ExtentManager::new(root, 8, Arc::downgrade(&fixture.ext2), 1).unwrap();

        assert!(manager.map_block(0).is_err());
    }

    #[ktest]
    fn failed_tree_growth_reclaims_the_new_data_block() {
        let fixture = Ext4FixtureBuilder::new(1, 2048)
            .with_free_blocks(1, 1)
            .block_bitmap(BlockBitmapInit::MetadataPlus(vec![40, 42, 44, 46]))
            .build()
            .unwrap();
        let root = inline_root(
            leaf_header(4),
            &[
                RawExtent::new(0, 1, 40),
                RawExtent::new(2, 1, 42),
                RawExtent::new(4, 1, 44),
                RawExtent::new(6, 1, 46),
            ],
        );
        let manager = ExtentManager::new(root, 32, Arc::downgrade(&fixture.ext2), 9).unwrap();
        let free_before = fixture.ext2.super_block().free_blocks_count();

        assert!(manager.allocate_range_blocks(8, 9).is_err());
        assert_eq!(fixture.ext2.super_block().free_blocks_count(), free_before);
        assert!(manager.map_block(8).unwrap().is_none());
    }

    #[ktest]
    fn rejects_extent_data_overlap_before_allocation() {
        let fixture = Ext4FixtureBuilder::new(1, 2048)
            .with_free_blocks(64, 64)
            .block_bitmap(BlockBitmapInit::MetadataPlus(vec![40, 41]))
            .build()
            .unwrap();
        let manager = ExtentManager::new(
            inline_root(
                leaf_header(2),
                &[RawExtent::new(0, 2, 40), RawExtent::new(4, 1, 41)],
            ),
            24,
            Arc::downgrade(&fixture.ext2),
            8,
        )
        .unwrap();
        let free_before = fixture.ext2.super_block().free_blocks_count();
        let root_before = manager.raw_block_ptrs().block_ptrs;

        assert_errno!(manager.allocate_range_blocks(6, 7), Errno::EUCLEAN);
        assert_eq!(fixture.ext2.super_block().free_blocks_count(), free_before);
        assert_eq!(manager.raw_block_ptrs().block_ptrs, root_before);
    }

    #[ktest]
    fn rejects_duplicate_external_extent_nodes_before_lookup() {
        let fixture = Ext4FixtureBuilder::new(1, 2048)
            .block_bitmap(BlockBitmapInit::MetadataPlus(vec![40, 41]))
            .build()
            .unwrap();
        let mut leaf = [0u8; BLOCK_SIZE];
        leaf[..ENTRY_SIZE].copy_from_slice(
            RawExtentHeader {
                magic: EXTENT_MAGIC,
                entries: 1,
                max: ((BLOCK_SIZE - ENTRY_SIZE) / ENTRY_SIZE) as u16,
                depth: 0,
                generation: 0,
            }
            .as_bytes(),
        );
        leaf[ENTRY_SIZE..2 * ENTRY_SIZE].copy_from_slice(RawExtent::new(0, 1, 41).as_bytes());
        fixture
            .disk
            .segment()
            .write_bytes(Bid::new(40).to_offset(), &leaf)
            .unwrap();
        let mut root = [0u32; RAW_BLOCK_PTRS_LEN];
        let bytes = root.as_mut_bytes();
        bytes[..ENTRY_SIZE].copy_from_slice(
            RawExtentHeader {
                magic: EXTENT_MAGIC,
                entries: 2,
                max: 4,
                depth: 1,
                generation: 0,
            }
            .as_bytes(),
        );
        bytes[ENTRY_SIZE..2 * ENTRY_SIZE].copy_from_slice(RawExtentIdx::new(0, 40).as_bytes());
        bytes[2 * ENTRY_SIZE..3 * ENTRY_SIZE].copy_from_slice(RawExtentIdx::new(1, 40).as_bytes());
        let manager = ExtentManager::new(root, 16, Arc::downgrade(&fixture.ext2), 2).unwrap();

        assert_errno!(manager.map_block(0), Errno::EUCLEAN);
    }

    #[ktest]
    fn rejects_extent_node_that_is_also_data_before_lookup() {
        let fixture = Ext4FixtureBuilder::new(1, 2048)
            .block_bitmap(BlockBitmapInit::MetadataPlus(vec![40]))
            .build()
            .unwrap();
        let mut leaf = [0u8; BLOCK_SIZE];
        leaf[..ENTRY_SIZE].copy_from_slice(
            RawExtentHeader {
                magic: EXTENT_MAGIC,
                entries: 1,
                max: ((BLOCK_SIZE - ENTRY_SIZE) / ENTRY_SIZE) as u16,
                depth: 0,
                generation: 0,
            }
            .as_bytes(),
        );
        leaf[ENTRY_SIZE..2 * ENTRY_SIZE].copy_from_slice(RawExtent::new(0, 1, 40).as_bytes());
        fixture
            .disk
            .segment()
            .write_bytes(Bid::new(40).to_offset(), &leaf)
            .unwrap();
        let mut root = [0u32; RAW_BLOCK_PTRS_LEN];
        let bytes = root.as_mut_bytes();
        bytes[..ENTRY_SIZE].copy_from_slice(
            RawExtentHeader {
                magic: EXTENT_MAGIC,
                entries: 1,
                max: 4,
                depth: 1,
                generation: 0,
            }
            .as_bytes(),
        );
        bytes[ENTRY_SIZE..2 * ENTRY_SIZE].copy_from_slice(RawExtentIdx::new(0, 40).as_bytes());
        let manager = ExtentManager::new(root, 8, Arc::downgrade(&fixture.ext2), 1).unwrap();

        assert_errno!(manager.map_block(0), Errno::EUCLEAN);
    }
}
