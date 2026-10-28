// SPDX-License-Identifier: MPL-2.0

use ostd::prelude::ktest;

use super::{
    ExtentTree, RawBlockPtrs, SECTORS_PER_BLOCK,
    mutation::ZERO_BATCH_BLOCKS,
    node::{
        ENTRY_SIZE, EXTENT_MAGIC, Extent, ExtentNode, NodeEntries, RawExtent, RawExtentHeader,
        RawExtentIdx,
    },
};
use crate::{
    fs::fs_impls::ext4::{
        fs::{Ext4, MountFlavor},
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
    bytes[ENTRY_SIZE..2 * ENTRY_SIZE].copy_from_slice(RawExtent::new(0, 1, data_bid).as_bytes());
    disk.segment()
        .write_bytes(Bid::new(bid.into()).to_offset(), &bytes)
        .unwrap();
}

#[ktest]
fn extent_lookup_visits_only_selected_path() {
    let f = extent_fixture_builder(1, 2048)
        .block_bitmap(BlockBitmapInit::MetadataPlus(vec![40, 41]))
        .build()
        .unwrap();
    write_leaf(&f.disk, 40, 0, 41);
    // An unreadable sibling must not affect a query in the first leaf.
    f.disk.fail_read_block(Some(42));
    let mut tree =
        ExtentTree::new(RawBlockPtrs::new(24, index_root(&[(0, 40), (100, 42)]))).unwrap();
    let reads = f.disk.block_reads(40);
    assert_eq!(tree.map_block(&f.fs, 0).unwrap(), Some(41));
    assert_eq!(f.disk.block_reads(40) - reads, 1);
    assert_eq!(f.disk.block_reads(42), 0);
    tree.lookup_run(&f.fs, 1, 100).unwrap();
    tree.node_cache.remove(40);
    f.disk.fail_read_block(Some(40));
    assert_eq!(tree.map_block(&f.fs, 0).unwrap(), Some(41));
    assert_eq!(tree.map_block(&f.fs, 1).unwrap(), None);
    assert_eq!(f.disk.block_reads(40), 1);
}

#[ktest]
fn extent_lookup_bounds_holes_and_checks_parent_keys() {
    use super::path::ExtentRun;

    let f = extent_fixture_builder(1, 2048).build().unwrap();
    let root = inline_root(
        leaf_header(2),
        &[RawExtent::new(0, 1, 40), RawExtent::new(100, 1, 41)],
    );
    let tree = ExtentTree::new(RawBlockPtrs::new(16, root)).unwrap();
    assert!(matches!(tree.lookup_run(&f.fs, 1, 200).unwrap(), ExtentRun::Hole(r) if r == (1..100)));
    assert!(
        matches!(tree.lookup_run(&f.fs, 101, 200).unwrap(), ExtentRun::Hole(r) if r == (101..200))
    );
    let root = inline_root(leaf_header(1), &[RawExtent::new(u32::MAX, 1, 40)]);
    let tree = ExtentTree::new(RawBlockPtrs::new(8, root)).unwrap();
    assert_eq!(tree.map_block(&f.fs, u32::MAX).unwrap(), Some(40));

    write_leaf(&f.disk, 40, 0, 41);
    let tree = ExtentTree::new(RawBlockPtrs::new(16, index_root(&[(10, 40)]))).unwrap();
    assert_errno!(tree.map_block(&f.fs, 10), Errno::EUCLEAN);
}

#[ktest]
fn extent_node_cache_retains_failed_writeback() {
    use super::{cache::ExtentNodeCache, node::ExtentNode};

    let f = extent_fixture_builder(2, 2048).build().unwrap();
    write_leaf(&f.disk, 40, 0, 41);
    let mut cache = ExtentNodeCache::new();
    let node = cache.load(&f.fs, 40, 0).unwrap();
    cache.load(&f.fs, 40, 0).unwrap();
    assert_eq!(f.disk.block_reads(40), 1);
    let changed = inline_root(leaf_header(1), &[RawExtent::new(0, 1, 42)]);
    cache.stage(
        40,
        Arc::new(ExtentNode::parse(changed.as_bytes(), Some(0)).unwrap()),
    );
    let sibling = Arc::new(ExtentNode::parse(changed.as_bytes(), Some(0)).unwrap());
    cache.stage(80, sibling);
    let parent = index_root(&[(0, 40)]);
    cache.stage(
        81,
        Arc::new(ExtentNode::parse(parent.as_bytes(), Some(1)).unwrap()),
    );
    f.disk.fail_write_block(Some(40));
    assert!(cache.sync(&f.fs).is_err());
    assert!(cache.is_dirty());
    // Submit siblings before waiting, even when one completes with an
    // error. A failed child batch must not submit its parent level.
    assert_eq!(f.disk.block_writes(80), 1);
    assert_eq!(f.disk.block_writes(81), 0);
    assert_eq!(
        cache.load(&f.fs, 40, 0).unwrap().first_key(),
        node.first_key()
    );
    // Clean-cache pressure must not evict authoritative dirty versions.
    for bid in 43..78 {
        write_leaf(&f.disk, bid, 0, 41);
        cache.load(&f.fs, bid, 0).unwrap();
    }
    let dirty_node = cache.load(&f.fs, 40, 0).unwrap();
    let NodeEntries::Leaf(entries) = &dirty_node.entries else {
        unreachable!()
    };
    assert_eq!(entries[0].start(), 42);
    assert_eq!(f.disk.block_reads(40), 1);
    f.disk.fail_write_block(None);
    cache.sync(&f.fs).unwrap();
    assert!(!cache.is_dirty());
    assert_eq!(f.disk.block_writes(40), 2);
    // Retain the entire failed batch for conservative retry.
    assert_eq!(f.disk.block_writes(80), 2);
    assert_eq!(f.disk.block_writes(81), 1);
    for bid in 43..78 {
        write_leaf(&f.disk, bid, 0, 41);
        cache.load(&f.fs, bid, 0).unwrap();
    }
    cache.load(&f.fs, 40, 0).unwrap();
    assert_eq!(f.disk.block_reads(40), 2);
}

#[ktest]
fn extent_deferred_reclaim_survives_owner_drop() {
    let f = extent_fixture_builder(1, 2048)
        .with_free_blocks(8, 8)
        .build()
        .unwrap();
    let before = f.fs.super_block().read_state().free_blocks_count();
    let range = f.fs.alloc_blocks(1, 0).unwrap();
    f.fs.defer_extent_blocks(vec![range]);
    f.fs.retry_extent_work().unwrap();
    assert_eq!(f.fs.super_block().read_state().free_blocks_count(), before);
    f.fs.retry_extent_work().unwrap();
    assert_eq!(f.fs.super_block().read_state().free_blocks_count(), before);
}

#[ktest]
fn extent_status_and_last_lookup_track_mutations() {
    use super::{path::ExtentRun, status::ExtentStatusTree};
    let mut status = ExtentStatusTree::default();
    assert!(status.lookup(7).is_none());
    status.record(ExtentRun::Hole(0..10));
    status.invalidate(5..6);
    assert!(status.lookup(4).is_none());
    for i in 0..513u64 {
        status.record(ExtentRun::Hole(i * 2..i * 2 + 1));
    }
    assert!(status.lookup(0).is_none());
    assert!(status.lookup(2).is_some());
    assert!(matches!(status.lookup(1024), Some(ExtentRun::Hole(_))));
    status.record(ExtentRun::Mapped {
        logical: 2000..2001,
        extent: Extent::new(2000, 1, 50),
    });
    status.record(ExtentRun::Mapped {
        logical: 2001..2002,
        extent: Extent::new(2001, 1, 51),
    });
    assert!(status.covers_initialized(2000..2002));
    assert!(!status.covers_initialized(1999..2002));
    let f = extent_fixture_builder(1, 2048)
        .with_free_blocks(64, 64)
        .build()
        .unwrap();
    let mut tree = ExtentTree::new(RawBlockPtrs::new(0, super::empty_extent_root())).unwrap();
    assert_eq!(tree.map_block(&f.fs, 2).unwrap(), None);
    tree.allocate_range_blocks(&f.fs, 2, 3).unwrap();
    let old = tree.map_block(&f.fs, 2).unwrap().unwrap();
    tree.allocate_range_blocks(&f.fs, 10, 11).unwrap();
    assert!(tree.lookup_cache.lock().lookup(2, 3).is_some());

    tree.truncate_to_byte_len(&f.fs, 0).unwrap();
    assert_eq!(tree.map_block(&f.fs, 2).unwrap(), None);
    let occupied = f.fs.alloc_blocks(1, old).unwrap();
    tree.allocate_range_blocks(&f.fs, 2, 3).unwrap();
    assert_ne!(tree.map_block(&f.fs, 2).unwrap(), Some(old));
    f.fs.free_blocks(occupied.start, 1).unwrap();
}

#[ktest]
fn extent_non_rightmost_split_leaves_sibling_unread_and_unwritten() {
    use super::node::{ExtentNode, NodeEntries};
    let f = extent_fixture_builder(1, 2048)
        .with_free_blocks(64, 64)
        .block_bitmap(BlockBitmapInit::MetadataPlus(vec![40, 41, 42, 43]))
        .build()
        .unwrap();
    for (bid, logical, data) in [(40u32, 0u32, 41u32), (42, 100, 43)] {
        let node = ExtentNode {
            depth: 0,
            capacity: 1,
            entries: NodeEntries::Leaf(vec![Extent::new(logical, 1, data)]),
        };
        let mut bytes = [0u8; BLOCK_SIZE];
        node.write_into(&mut bytes);
        f.disk
            .segment()
            .write_bytes(Bid::new(u64::from(bid)).to_offset(), &bytes)
            .unwrap();
    }
    let mut tree =
        ExtentTree::new(RawBlockPtrs::new(32, index_root(&[(0, 40), (100, 42)]))).unwrap();
    f.disk.fail_read_block(Some(42));
    tree.allocate_range_blocks(&f.fs, 2, 3).unwrap();
    assert_eq!(f.disk.block_reads(42), 0);
    tree.sync_metadata(&f.fs).unwrap();
    assert_eq!(f.disk.block_writes(42), 0);
    assert_eq!(tree.map_block(&f.fs, 0).unwrap(), Some(41));
    assert!(tree.map_block(&f.fs, 2).unwrap().is_some());
    f.disk.fail_read_block(None);
    assert_eq!(tree.map_block(&f.fs, 100).unwrap(), Some(43));
}

#[ktest]
fn extent_small_nodes_split_ancestors_and_reject_depth_overflow() {
    for depth in [1u16, 2, 3, 4, 5] {
        let f = extent_fixture_builder(1, 2048)
            .with_free_blocks(64, 64)
            .block_bitmap(BlockBitmapInit::MetadataPlus(vec![40, 41, 42, 43, 44, 50]))
            .build()
            .unwrap();
        for level in 0..depth {
            let mut bytes = [0u8; BLOCK_SIZE];
            let header = RawExtentHeader {
                entries: 1,
                max: 1,
                depth: depth - level - 1,
                ..leaf_header(0)
            };
            bytes[..ENTRY_SIZE].copy_from_slice(header.as_bytes());
            if header.depth == 0 {
                bytes[ENTRY_SIZE..2 * ENTRY_SIZE]
                    .copy_from_slice(RawExtent::new(0, 1, 50).as_bytes());
            } else {
                bytes[ENTRY_SIZE..2 * ENTRY_SIZE]
                    .copy_from_slice(RawExtentIdx::new(0, 41 + u32::from(level)).as_bytes());
            }
            f.disk
                .segment()
                .write_bytes(Bid::new(u64::from(40 + level)).to_offset(), &bytes)
                .unwrap();
        }
        let mut root = index_root(&[(0, 40)]);
        root.as_mut_bytes()[6..8].copy_from_slice(&depth.to_ne_bytes());
        root.as_mut_bytes()[4..6].copy_from_slice(&1u16.to_ne_bytes());
        let mut tree = ExtentTree::new(RawBlockPtrs::new(
            (u32::from(depth) + 1) * SECTORS_PER_BLOCK,
            root,
        ))
        .unwrap();
        let before = f.fs.super_block().read_state().free_blocks_count();
        if depth == 5 {
            assert_errno!(tree.allocate_range_blocks(&f.fs, 2, 3), Errno::EFBIG);
            assert_eq!(tree.raw_block_ptrs().block_ptrs, root);
            assert_eq!(f.fs.super_block().read_state().free_blocks_count(), before);
        } else {
            tree.allocate_range_blocks(&f.fs, 2, 3).unwrap();
            assert_eq!(tree.map_block(&f.fs, 0).unwrap(), Some(50));
            assert!(tree.map_block(&f.fs, 2).unwrap().is_some());
            tree.sync_metadata(&f.fs).unwrap();
            let reloaded = ExtentTree::new(tree.raw_block_ptrs()).unwrap();
            assert!(reloaded.map_block(&f.fs, 2).unwrap().is_some());
        }
    }
}

#[ktest]
fn extent_update_failure_keeps_dirty_nodes_for_retry() {
    let f = extent_fixture_builder(1, 2048)
        .with_free_blocks(64, 64)
        .build()
        .unwrap();
    let mut tree = ExtentTree::new(RawBlockPtrs::new(0, super::empty_extent_root())).unwrap();
    for i in 0..5 {
        tree.allocate_range_blocks(&f.fs, i * 3, i * 3 + 1).unwrap();
    }
    let bid = tree.raw_block_ptrs().block_ptrs[4];
    f.disk.fail_write_block(Some(bid as usize));
    assert!(tree.sync_metadata(&f.fs).is_err());
    assert!(tree.is_dirty());
    assert!(tree.map_block(&f.fs, 12).unwrap().is_some());
    f.disk.fail_write_block(None);
    tree.sync_metadata(&f.fs).unwrap();
    let restored = ExtentTree::new(tree.raw_block_ptrs()).unwrap();
    assert!(restored.map_block(&f.fs, 12).unwrap().is_some());
    assert_eq!(f.disk.block_writes(bid as usize), 2);
    tree.clear_dirty();
    assert!(!tree.is_dirty());
}

#[ktest]
fn extent_local_updates_split_prune_and_reload() {
    let f = extent_fixture_builder(1, 2048)
        .with_blocks_per_group(2048)
        .with_free_blocks(1800, 1800)
        .build()
        .unwrap();
    let before = f.fs.super_block().read_state().free_blocks_count();
    let mut tree = ExtentTree::new(RawBlockPtrs::new(0, super::empty_extent_root())).unwrap();
    for i in 0..350usize {
        tree.allocate_range_blocks(&f.fs, i * 3, i * 3 + 1).unwrap();
    }
    // Node modifications remain in memory until explicit metadata sync.
    assert!(tree.node_cache.is_dirty());
    tree.sync_metadata(&f.fs).unwrap();
    let mut restored = ExtentTree::new(tree.raw_block_ptrs()).unwrap();
    for i in 0..350u32 {
        assert!(restored.map_block(&f.fs, i * 3).unwrap().is_some());
        assert_eq!(restored.map_block(&f.fs, i * 3 + 1).unwrap(), None);
    }
    let retained_bid = restored.raw_block_ptrs().block_ptrs[4];
    restored.node_cache.remove(retained_bid);
    f.disk.fail_read_block(Some(retained_bid as usize));
    restored
        .truncate_to_byte_len(&f.fs, 900 * BLOCK_SIZE)
        .unwrap();
    f.disk.fail_read_block(None);
    restored
        .truncate_to_byte_len(&f.fs, 7 * BLOCK_SIZE)
        .unwrap();
    assert_eq!(
        restored.raw_block_ptrs().sector_count,
        3 * SECTORS_PER_BLOCK
    );
    restored.truncate_to_byte_len(&f.fs, 0).unwrap();
    assert_eq!(f.fs.super_block().read_state().free_blocks_count(), before);
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
    let mut tree = ExtentTree::new(RawBlockPtrs::new(0, inline_root(leaf_header(0), &[]))).unwrap();
    assert!(!tree.is_dirty());
    assert_eq!(tree.map_block(&fixture.fs, 0).unwrap(), None);

    // Adjacent allocations merge, preserve existing mappings, and zero data.
    tree.allocate_range_blocks(&fixture.fs, 0, 2).unwrap();
    tree.allocate_range_blocks(&fixture.fs, 2, 4).unwrap();
    // Mapping validation uses geometry and protected metadata ranges,
    // without allocation-bit scans or the global counter lock.
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
    tree.sync_metadata(&fixture.fs).unwrap();
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
    let mut tree = ExtentTree::new(RawBlockPtrs::new(0, inline_root(leaf_header(0), &[]))).unwrap();
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
fn unwritten_extent_reads_zero_and_buffered_preparation_initializes_only_target() {
    let f = extent_fixture_builder(1, 2048)
        .block_bitmap(BlockBitmapInit::MetadataPlus(vec![40, 41, 42, 43]))
        .build()
        .unwrap();
    let root = inline_root(leaf_header(1), &[RawExtent::new(0, 32768 + 4, 40)]);
    let mut tree = ExtentTree::new(RawBlockPtrs::new(32, root)).unwrap();
    assert_eq!(tree.map_block(&f.fs, 1).unwrap(), None);
    let reloaded = ExtentTree::new(tree.raw_block_ptrs()).unwrap();
    assert_eq!(reloaded.map_block(&f.fs, 1).unwrap(), None);
    tree.allocate_range_blocks(&f.fs, 1, 3).unwrap();
    assert_eq!(tree.map_block(&f.fs, 0).unwrap(), None);
    assert_eq!(tree.map_block(&f.fs, 1).unwrap(), Some(41));
    assert_eq!(tree.map_block(&f.fs, 2).unwrap(), Some(42));
    assert_eq!(tree.map_block(&f.fs, 3).unwrap(), None);
    let reloaded = ExtentTree::new(tree.raw_block_ptrs()).unwrap();
    assert_eq!(reloaded.map_block(&f.fs, 1).unwrap(), Some(41));
    assert_eq!(reloaded.map_block(&f.fs, 3).unwrap(), None);
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
fn extent_tree_rejects_wrong_child_depth_before_mutation() {
    let f = extent_fixture_builder(1, 2048)
        .with_free_blocks(64, 64)
        .build()
        .unwrap();
    write_leaf(&f.disk, 40, 1, 41);
    let root = index_root(&[(0, 40)]);
    let mut tree = ExtentTree::new(RawBlockPtrs::new(24, root)).unwrap();
    assert_errno!(tree.allocate_range_blocks(&f.fs, 6, 7), Errno::EUCLEAN);
    assert_eq!(tree.raw_block_ptrs().block_ptrs, root);
    assert!(!tree.is_dirty());
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
    let candidate = fixture.fs.alloc_blocks(1, 0).unwrap();
    fixture.fs.free_blocks(candidate.start, 1).unwrap();
    fixture
        .disk
        .fail_write_block(Some(candidate.start as usize));
    assert!(tree.allocate_range_blocks(&fixture.fs, 8, 9).is_err());
    assert_eq!(tree.raw_block_ptrs().block_ptrs, root);
    assert_eq!(
        fixture.fs.super_block().read_state().free_blocks_count(),
        free_before
    );
    fixture.disk.fail_write_block(None);
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
impl ExtentTree {
    /// Returns the mapped physical run beginning at `iblock`.
    ///
    /// The returned range is capped at `max_blocks` and at the end of the
    /// containing extent. `None` denotes a hole at `iblock`.
    fn mapped_run(
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

    fn extents(&self, fs: &Ext4) -> Result<Vec<Extent>> {
        fn visit(
            tree: &ExtentTree,
            fs: &Ext4,
            node: &ExtentNode,
            out: &mut Vec<Extent>,
        ) -> Result<()> {
            match &node.entries {
                NodeEntries::Leaf(entries) => out.extend_from_slice(entries),
                NodeEntries::Index(entries) => {
                    for e in entries {
                        visit(
                            tree,
                            fs,
                            tree.node_cache.load(fs, e.leaf(), node.depth - 1)?.as_ref(),
                            out,
                        )?;
                    }
                }
            }
            Ok(())
        }
        let mut out = Vec::new();
        visit(
            self,
            fs,
            &ExtentNode::parse(self.raw_block_ptrs.block_ptrs.as_bytes(), None)?,
            &mut out,
        )?;
        Ok(out)
    }
}
