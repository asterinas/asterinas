// SPDX-License-Identifier: MPL-2.0

//! Per-inode EXT4 extent mapping.

use ostd::mm::io::util::HasVmReaderWriter;

use super::RawBlockPtrs;
use crate::fs::fs_impls::ext4::{fs::Ext4, inode::RAW_BLOCK_PTRS_LEN, prelude::*};

mod node;
mod tree;

/// An inode's EXT4 extent mapping and sector accounting.
///
/// The root occupies the inode's 60-byte `i_block` field. A depth-zero root
/// contains extents directly; a deeper root contains indexes to child nodes.
/// The layouts supported by the current writer are:
///
/// ```text
/// inode.i_block
/// +------------------+     depth = 0
/// | header | extents |--------------------------> data blocks
/// +------------------+
///
/// +------------------+     depth = 1     leaf blocks
/// | header | indexes |--------------> +------------------+
/// +------------------+                | header | extents |--> data blocks
///                                     +------------------+
/// ```
///
/// Extents are ordered and do not overlap logically or physically; metadata
/// blocks must be allocated data-area blocks distinct from extent data blocks.
/// New data blocks are zeroed before their mappings become visible.
/// `raw_block_ptrs` holds the root and the count of data and tree blocks in
/// 512-byte sectors; `dirty` tracks changes to this inode metadata, not file data.
/// The parent [`BlockMapping`](super::BlockMapping) lock protects queries and
/// serializes mapping changes, block allocation/release, and accounting updates.
///
/// Writes rebuild depth-zero or depth-one trees with initialized extents and
/// 32-bit physical block addresses. Metadata updates are not journaled or
/// crash-atomic.
#[derive(Debug)]
pub(in crate::fs::fs_impls::ext4::inode) struct ExtentTree {
    raw_block_ptrs: RawBlockPtrs,
    dirty: bool,
}

impl ExtentTree {
    /// Creates an extent mapping from an inode-resident root and sector count.
    ///
    /// This validates the inline root. External nodes are loaded and validated
    /// when an operation traverses or rewrites the tree.
    pub(super) fn new(raw_block_ptrs: RawBlockPtrs) -> Result<Self> {
        Self::validate_root(&raw_block_ptrs.block_ptrs)?;
        Ok(Self {
            raw_block_ptrs,
            dirty: false,
        })
    }

    /// Resolves one logical block to a physical block.
    ///
    /// Returns `None` when the logical block lies in a hole.
    pub(super) fn map_block(&self, fs: &Ext4, iblock: Iblock) -> Result<Option<Ext4Bid>> {
        Ok(self
            .find(fs, iblock)?
            .map(|extent| extent.physical_block(iblock)))
    }

    /// Returns the mapped physical run beginning at `iblock`.
    ///
    /// The returned range is capped at `max_blocks` and at the end of the
    /// containing extent. `None` denotes a hole at `iblock`.
    pub(in crate::fs::fs_impls::ext4::inode) fn mapped_run(
        &self,
        fs: &Ext4,
        iblock: Iblock,
        max_blocks: u32,
    ) -> Result<Option<Range<Ext4Bid>>> {
        let Some(extent) = self.find(fs, iblock)? else {
            return Ok(None);
        };
        let offset = iblock - extent.block();
        let len = (u32::from(extent.len()) - offset).min(max_blocks);
        let start = extent.physical_block(iblock);
        Ok(Some(start..start + len))
    }

    /// Returns the extent root and sector count to store in the raw inode.
    pub(super) fn raw_block_ptrs(&self) -> RawBlockPtrs {
        self.raw_block_ptrs
    }

    /// Returns whether the inode-resident extent metadata needs writeback.
    pub(super) fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Marks the inode-resident extent metadata as written back.
    pub(super) fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    /// Allocates and publishes mappings for holes in `[start, end)`.
    ///
    /// Existing mappings are preserved. New blocks are zeroed before tree
    /// replacement so that a newly visible mapping cannot expose stale device
    /// data. Failures before replacement release blocks allocated by this call.
    pub(super) fn allocate_range_blocks(
        &mut self,
        fs: &Ext4,
        start: usize,
        end: usize,
    ) -> Result<()> {
        let start = Iblock::try_from(start)
            .map_err(|_| Error::with_message(Errno::EINVAL, "logical block number overflow"))?;
        let end = Iblock::try_from(end)
            .map_err(|_| Error::with_message(Errno::EINVAL, "logical block number overflow"))?;
        if start >= end {
            return Ok(());
        }

        let existing = self.extents(fs)?;
        let mut allocated = Vec::new();
        let mut additions = Vec::new();
        for hole in holes(&existing, start, end) {
            let mut iblock = hole.start;
            while iblock < hole.end {
                let requested = (hole.end - iblock).min(32768);
                let range = match fs.alloc_blocks(requested, 0) {
                    Ok(range) => range,
                    Err(error) => {
                        free_ranges(fs, &allocated);
                        return Err(error);
                    }
                };
                let len = range.len() as u32;
                if let Err(error) = zero_new_blocks(fs, &range) {
                    let _ = fs.free_blocks(range.start, len);
                    free_ranges(fs, &allocated);
                    return Err(error);
                }
                let extent_len = match u16::try_from(len) {
                    Ok(len) if len <= 32768 => len,
                    _ => {
                        let _ = fs.free_blocks(range.start, len);
                        free_ranges(fs, &allocated);
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
        let anticipated = match self.anticipated_rebuild_delta(&replacement) {
            Ok(delta) => delta,
            Err(error) => {
                free_ranges(fs, &allocated);
                return Err(error);
            }
        };
        let data_blocks = allocated.iter().map(|range| range.len() as u32).sum();
        let mut sector_count = self.raw_block_ptrs.sector_count;
        if let Err(error) = add_sectors(
            &mut sector_count,
            data_blocks,
            tree::TreeDelta {
                allocated: anticipated.allocated,
                freed: 0,
            },
        ) {
            free_ranges(fs, &allocated);
            return Err(error);
        }
        let (root, delta) = match self.rebuild(fs, &replacement) {
            Ok(delta) => delta,
            Err(error) => {
                free_ranges(fs, &allocated);
                return Err(error);
            }
        };
        sector_count = self.raw_block_ptrs.sector_count;
        add_sectors(&mut sector_count, data_blocks, delta)?;
        self.raw_block_ptrs = RawBlockPtrs::new(sector_count, root);
        self.dirty = true;
        Ok(())
    }

    /// Removes mappings beyond the block containing `new_size`.
    ///
    /// Replacement metadata is prepared before obsolete data blocks are released.
    /// The candidate root and sector accounting are recorded after the release
    /// attempts, leaving the mapping dirty even if a release fails. Unlike
    /// the legacy block-pointer implementation, this path propagates errors so a
    /// foreground truncate cannot report success while extent replacement failed.
    pub(super) fn truncate_to_byte_len(&mut self, fs: &Ext4, new_size: usize) -> Result<()> {
        let keep_blocks = Iblock::try_from(new_size.div_ceil(BLOCK_SIZE))
            .map_err(|_| Error::with_message(Errno::EINVAL, "logical block number overflow"))?;
        let extents = self.extents(fs)?;
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

        let anticipated = self.anticipated_rebuild_delta(&kept)?;
        let planned_data_blocks = freed.iter().map(|range| range.len() as u32).sum();
        let mut sector_count = self.raw_block_ptrs.sector_count;
        subtract_sectors(&mut sector_count, planned_data_blocks, anticipated)?;

        let (root, delta) = self.rebuild(fs, &kept)?;
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
        sector_count = self.raw_block_ptrs.sector_count;
        subtract_sectors(&mut sector_count, data_blocks, delta)?;
        self.raw_block_ptrs = RawBlockPtrs::new(sector_count, root);
        self.dirty = true;
        if let Some(error) = release_error {
            return Err(error);
        }
        Ok(())
    }
}

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
    ExtentTree::validate_root(&root)
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

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    use super::{
        ExtentTree, RawBlockPtrs, SECTORS_PER_BLOCK, ZERO_BATCH_BLOCKS,
        node::{EXTENT_MAGIC, RawExtent, RawExtentHeader, RawExtentIdx},
        tree::ENTRY_SIZE,
    };
    use crate::{
        fs::fs_impls::ext4::{
            fs::MountFlavor,
            inode::RAW_BLOCK_PTRS_LEN,
            prelude::*,
            test_utils::{
                BlockBitmapInit, Ext4FixtureBuilder, Ext4MemoryDisk, assert_errno, group0_layout,
                make_valid_super_block,
            },
        },
        time::clocks,
    };

    fn extent_fixture_builder(groups: u32, nblocks: usize) -> Ext4FixtureBuilder {
        Ext4FixtureBuilder::new(groups, nblocks).with_mount_flavor(MountFlavor::Ext4)
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

    fn index_root(indexes: &[(Iblock, Ext4Bid)]) -> [u32; RAW_BLOCK_PTRS_LEN] {
        let mut root = inline_root(
            RawExtentHeader {
                entries: indexes.len() as u16,
                depth: 1,
                ..leaf_header(0)
            },
            &[],
        );
        for (index, &(logical, physical)) in indexes.iter().enumerate() {
            let offset = ENTRY_SIZE * (index + 1);
            root.as_mut_bytes()[offset..offset + ENTRY_SIZE]
                .copy_from_slice(RawExtentIdx::new(logical, physical).as_bytes());
        }
        root
    }

    fn write_leaf(disk: &Ext4MemoryDisk, bid: Ext4Bid, depth: u16, data_bid: Ext4Bid) {
        let mut bytes = [0u8; BLOCK_SIZE];
        let header = RawExtentHeader {
            entries: 1,
            max: ((BLOCK_SIZE - ENTRY_SIZE) / ENTRY_SIZE) as u16,
            depth,
            ..leaf_header(0)
        };
        bytes[..ENTRY_SIZE].copy_from_slice(header.as_bytes());
        bytes[ENTRY_SIZE..2 * ENTRY_SIZE]
            .copy_from_slice(RawExtent::new(0, 1, data_bid).as_bytes());
        disk.segment()
            .write_bytes(Bid::new(bid.into()).to_offset(), &bytes)
            .unwrap();
    }

    #[ktest]
    fn extent_tree_allocation_lookup_reload_and_truncate() {
        clocks::init_for_ktest();
        let layout = group0_layout(&make_valid_super_block(1));
        let fixture = extent_fixture_builder(1, 2048)
            .with_free_blocks(64, 64)
            .block_bitmap(BlockBitmapInit::MetadataPlus(vec![
                layout.first_data_bid,
                layout.first_data_bid + 1,
            ]))
            .build()
            .unwrap();
        let first_bid = layout.first_data_bid + 2;
        let stale = [0xa5; BLOCK_SIZE];
        for bid in first_bid..first_bid + 4 {
            fixture
                .disk
                .segment()
                .write_bytes(Bid::new(bid.into()).to_offset(), &stale)
                .unwrap();
        }
        let free_before = fixture.fs.super_block().read_state().free_blocks_count();
        let mut tree =
            ExtentTree::new(RawBlockPtrs::new(0, inline_root(leaf_header(0), &[]))).unwrap();
        assert!(!tree.is_dirty());
        assert_eq!(tree.map_block(&fixture.fs, 0).unwrap(), None);

        // Adjacent allocations merge, preserve existing mappings, and zero data.
        tree.allocate_range_blocks(&fixture.fs, 0, 2).unwrap();
        tree.allocate_range_blocks(&fixture.fs, 2, 4).unwrap();
        // Mapping validation only needs immutable geometry and group-local
        // allocation state, even while the global counters are write-locked.
        {
            let _state = fixture.fs.super_block().write_state();
            assert_eq!(tree.map_block(&fixture.fs, 2).unwrap(), Some(first_bid + 2));
            assert_eq!(fixture.fs.max_file_size(), fixture.sb.max_file_size());
        }
        assert_eq!(tree.extents(&fixture.fs).unwrap().len(), 1);
        assert_eq!(
            tree.mapped_run(&fixture.fs, 1, 100).unwrap(),
            Some(first_bid + 1..first_bid + 4)
        );
        assert_eq!(
            tree.mapped_run(&fixture.fs, 1, 1).unwrap(),
            Some(first_bid + 1..first_bid + 2)
        );
        for logical in 0..4 {
            assert_eq!(
                tree.map_block(&fixture.fs, logical).unwrap(),
                Some(first_bid + logical)
            );
            let mut contents = [0xff; BLOCK_SIZE];
            fixture
                .disk
                .segment()
                .read_bytes(
                    Bid::new((first_bid + logical).into()).to_offset(),
                    &mut contents,
                )
                .unwrap();
            assert_eq!(contents, [0; BLOCK_SIZE]);
        }

        // Sparse allocations cross the inline-root boundary and require a leaf.
        let mut sparse = Vec::new();
        for logical in [8u32, 12, 16, 20] {
            tree.allocate_range_blocks(&fixture.fs, logical as usize, logical as usize + 1)
                .unwrap();
            sparse.push((
                logical,
                tree.map_block(&fixture.fs, logical).unwrap().unwrap(),
            ));
        }
        assert_eq!(tree.extents(&fixture.fs).unwrap().len(), 5);
        assert_eq!(tree.raw_block_ptrs().sector_count, 9 * SECTORS_PER_BLOCK);
        assert_eq!(
            fixture.fs.super_block().read_state().free_blocks_count(),
            free_before - 9
        );
        assert!(tree.is_dirty());
        for hole in [4, 7, 9, 11, 13, 19, 21] {
            assert_eq!(tree.map_block(&fixture.fs, hole).unwrap(), None);
        }
        // Reconstruct from the inode root and external leaf on the fixture disk.
        let mut restored = ExtentTree::new(tree.raw_block_ptrs()).unwrap();
        for (logical, physical) in sparse {
            assert_eq!(
                restored.map_block(&fixture.fs, logical).unwrap(),
                Some(physical)
            );
        }
        assert_eq!(
            restored.mapped_run(&fixture.fs, 0, 100).unwrap(),
            Some(first_bid..first_bid + 4)
        );

        // Shrinking releases the leaf, preserves the partial final block, then
        // releases all remaining data and restores the original free count.
        restored
            .truncate_to_byte_len(&fixture.fs, 2 * BLOCK_SIZE + 1)
            .unwrap();
        assert_eq!(
            restored.raw_block_ptrs().sector_count,
            3 * SECTORS_PER_BLOCK
        );
        assert_eq!(
            fixture.fs.super_block().read_state().free_blocks_count(),
            free_before - 3
        );
        assert_eq!(
            restored.map_block(&fixture.fs, 2).unwrap(),
            Some(first_bid + 2)
        );
        for logical in [3, 8, 12, 16, 20] {
            assert_eq!(restored.map_block(&fixture.fs, logical).unwrap(), None);
        }
        restored.clear_dirty();
        assert!(!restored.is_dirty());
        restored.truncate_to_byte_len(&fixture.fs, 0).unwrap();
        assert!(restored.is_dirty());
        assert_eq!(restored.raw_block_ptrs().sector_count, 0);
        assert_eq!(restored.map_block(&fixture.fs, 0).unwrap(), None);
        assert_eq!(
            fixture.fs.super_block().read_state().free_blocks_count(),
            free_before
        );
    }

    #[ktest]
    fn extent_tree_large_allocation_zeroing_and_reclamation() {
        let fixture = extent_fixture_builder(1, 512)
            .with_blocks_per_group(512)
            .with_free_blocks(300, 300)
            .build()
            .unwrap();
        let free_before = fixture.fs.super_block().read_state().free_blocks_count();
        let mut tree =
            ExtentTree::new(RawBlockPtrs::new(0, inline_root(leaf_header(0), &[]))).unwrap();
        tree.allocate_range_blocks(&fixture.fs, 0, 257).unwrap();
        assert!(fixture.disk.max_write_blocks() <= ZERO_BATCH_BLOCKS);
        assert_eq!(tree.raw_block_ptrs().sector_count, 257 * SECTORS_PER_BLOCK);
        for logical in [0, 255, 256] {
            let physical = tree.map_block(&fixture.fs, logical).unwrap().unwrap();
            let mut contents = [0xff; BLOCK_SIZE];
            fixture
                .disk
                .segment()
                .read_bytes(Bid::new(physical.into()).to_offset(), &mut contents)
                .unwrap();
            assert_eq!(contents, [0; BLOCK_SIZE]);
        }
        tree.truncate_to_byte_len(&fixture.fs, 0).unwrap();
        assert_eq!(
            fixture.fs.super_block().read_state().free_blocks_count(),
            free_before
        );
        assert_eq!(tree.raw_block_ptrs().sector_count, 0);
    }

    #[ktest]
    fn extent_tree_rejects_invalid_root_formats() {
        let cases = [
            (
                inline_root(
                    RawExtentHeader {
                        magic: 0,
                        ..leaf_header(0)
                    },
                    &[],
                ),
                Errno::EUCLEAN,
            ),
            (
                inline_root(
                    leaf_header(2),
                    &[RawExtent::new(2, 4, 100), RawExtent::new(5, 1, 200)],
                ),
                Errno::EUCLEAN,
            ),
            (
                inline_root(
                    RawExtentHeader {
                        depth: 6,
                        ..leaf_header(0)
                    },
                    &[],
                ),
                Errno::EUCLEAN,
            ),
            (
                inline_root(
                    leaf_header(1),
                    &[RawExtent {
                        block: 0,
                        len: 1,
                        start_hi: 1,
                        start_lo: 0,
                    }],
                ),
                Errno::EOPNOTSUPP,
            ),
        ];
        for (root, error) in cases {
            assert_errno!(ExtentTree::new(RawBlockPtrs::new(0, root)), error);
        }
    }

    #[ktest]
    fn extent_tree_rejects_invalid_data_and_metadata_references() {
        // Validate both inline data references and index references before use.
        for index in [false, true] {
            for location in 0..3 {
                let fixture = extent_fixture_builder(1, 2048)
                    .block_bitmap(BlockBitmapInit::MetadataPlus(vec![40]))
                    .build()
                    .unwrap();
                let bid = match location {
                    0 => fixture.sb.total_blocks(),
                    1 => group0_layout(&fixture.sb).block_bitmap_bid,
                    _ => fixture.sb.group_descriptors_bid(0),
                };
                let root = if index {
                    // A syntactically valid node must still be rejected when its
                    // address points outside data space or into system metadata.
                    write_leaf(&fixture.disk, bid, 0, 40);
                    index_root(&[(0, bid)])
                } else {
                    inline_root(leaf_header(1), &[RawExtent::new(0, 1, bid)])
                };
                let mut tree = ExtentTree::new(RawBlockPtrs::new(8, root)).unwrap();
                let free_before = fixture.fs.super_block().read_state().free_blocks_count();
                assert_errno!(tree.map_block(&fixture.fs, 0), Errno::EUCLEAN);
                assert_errno!(
                    tree.allocate_range_blocks(&fixture.fs, 2, 3),
                    Errno::EUCLEAN
                );
                assert_eq!(
                    fixture.fs.super_block().read_state().free_blocks_count(),
                    free_before
                );
                assert_eq!(tree.raw_block_ptrs().block_ptrs, root);
                assert_eq!(tree.raw_block_ptrs().sector_count, 8);
                assert!(!tree.is_dirty());
            }
        }
    }

    #[ktest]
    fn extent_tree_rejects_corrupt_ownership_and_child_depth() {
        for case in 0..4 {
            let fixture = extent_fixture_builder(1, 2048)
                .with_free_blocks(64, 64)
                .block_bitmap(BlockBitmapInit::MetadataPlus(vec![40, 41]))
                .build()
                .unwrap();
            let root = match case {
                0 => inline_root(
                    leaf_header(2),
                    &[RawExtent::new(0, 2, 40), RawExtent::new(4, 1, 41)],
                ),
                1 => {
                    write_leaf(&fixture.disk, 40, 0, 41);
                    index_root(&[(0, 40), (1, 40)])
                }
                2 => {
                    write_leaf(&fixture.disk, 40, 0, 40);
                    index_root(&[(0, 40)])
                }
                _ => {
                    write_leaf(&fixture.disk, 40, 1, 41);
                    index_root(&[(0, 40)])
                }
            };
            let mut tree = ExtentTree::new(RawBlockPtrs::new(24, root)).unwrap();
            let free_before = fixture.fs.super_block().read_state().free_blocks_count();
            assert_errno!(tree.map_block(&fixture.fs, 0), Errno::EUCLEAN);
            assert_errno!(
                tree.allocate_range_blocks(&fixture.fs, 6, 7),
                Errno::EUCLEAN
            );
            assert_eq!(tree.raw_block_ptrs().block_ptrs, root);
            assert_eq!(tree.raw_block_ptrs().sector_count, 24);
            assert!(!tree.is_dirty());
            assert_eq!(
                fixture.fs.super_block().read_state().free_blocks_count(),
                free_before
            );
        }
    }

    #[ktest]
    fn extent_tree_growth_failure_preserves_mapping_and_accounting() {
        let fixture = extent_fixture_builder(1, 2048)
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
        let mut tree = ExtentTree::new(RawBlockPtrs::new(32, root)).unwrap();
        let free_before = fixture.fs.super_block().read_state().free_blocks_count();
        assert_errno!(tree.allocate_range_blocks(&fixture.fs, 8, 9), Errno::ENOSPC);
        assert_eq!(
            fixture.fs.super_block().read_state().free_blocks_count(),
            free_before
        );
        assert_eq!(tree.raw_block_ptrs().block_ptrs, root);
        assert_eq!(tree.raw_block_ptrs().sector_count, 32);
        assert!(!tree.is_dirty());
        for (logical, physical) in [(0, 40), (2, 42), (4, 44), (6, 46)] {
            assert_eq!(
                tree.map_block(&fixture.fs, logical).unwrap(),
                Some(physical)
            );
        }
        assert_eq!(tree.map_block(&fixture.fs, 8).unwrap(), None);
    }
}
