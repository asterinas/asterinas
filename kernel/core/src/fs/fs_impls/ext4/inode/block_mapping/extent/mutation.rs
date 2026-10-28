// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(block_mapping)]

use ostd::mm::io::util::HasVmReaderWriter;

use super::{
    AllocationMode, ExtentTree, RawBlockPtrs, SECTORS_PER_BLOCK,
    cache::READ_BATCH_NODES,
    node::{
        EXTERNAL_NODE_CAPACITY, Extent, ExtentIdx, ExtentNode, INLINE_NODE_CAPACITY,
        MAX_EXTENT_DEPTH, MAX_INITIALIZED_EXTENT_LEN, MAX_UNWRITTEN_EXTENT_LEN, NodeEntries,
    },
    path::{ExtentMappingRange, ExtentPath},
};
use crate::fs::fs_impls::ext4::{fs::Ext4, prelude::*, reclaim::DeferredReclaim};

/// A candidate mapping owns every new allocation until it is accepted.
/// Dropping a rejected candidate releases memory-resident allocation bits;
/// failed releases transfer ownership to the filesystem retry queue.
pub(in block_mapping) struct PreparedUpdate<'a> {
    root: ExtentNode,
    nodes: BTreeMap<Ext4Bid, Arc<ExtentNode>>,
    removed: Vec<Ext4Bid>,
    retired: Vec<Range<Ext4Bid>>,
    allocations: AllocatedBlocks<'a>,
    sectors: u32,
    invalidated: Range<Iblock>,
}

/// Owns new blocks until the prepared mapping accepts them.
struct AllocatedBlocks<'a> {
    fs: &'a Ext4,
    ranges: Vec<Range<Ext4Bid>>,
}

/// The effect of suffix pruning on one node.
enum PruneResult {
    Unchanged,
    Removed,
    Replaced(ExtentNode),
}

impl ExtentTree {
    /// Ensures the range is allocated and initialized, allocating zeroed blocks
    /// for holes and zeroing existing unwritten ranges. Initialized mappings
    /// are preserved; earlier successful updates remain if a later update fails.
    pub(in block_mapping) fn allocate_range_blocks(
        &mut self,
        fs: &Ext4,
        start: usize,
        end: usize,
    ) -> Result<()> {
        self.allocate_range(fs, start, end, AllocationMode::Initialized)
    }

    pub(in block_mapping) fn allocate_range(
        &mut self,
        fs: &Ext4,
        start: usize,
        end: usize,
        mode: AllocationMode,
    ) -> Result<()> {
        let unwritten = mode == AllocationMode::Unwritten;
        if end as u128 > u128::from(Iblock::MAX) || start > end {
            return_errno_with_message!(Errno::EINVAL, "invalid logical allocation range");
        }
        let (start, end) = (start as Iblock, end as Iblock);
        if self.lookup_cache.lock().covers_initialized(start..end) {
            return Ok(());
        }
        let mut current = start;
        while current < end {
            match self.lookup_mapping_range(fs, current, end)? {
                ExtentMappingRange::Mapped { logical, extent } => {
                    if !unwritten && extent.is_unwritten() {
                        let bid = extent.physical_block(current);
                        zero_new_blocks(fs, &(bid..bid + (logical.end - current)))?;
                        self.mark_written(fs, current as usize, logical.end as usize)?;
                    }
                    current = logical.end;
                }
                ExtentMappingRange::Hole(hole) => {
                    let path = self.lookup_path(fs, current)?;
                    let mut update = PreparedUpdate::new(
                        fs,
                        (*path.levels[0].node).clone(),
                        self.raw_block_ptrs.sector_count,
                    );
                    let max_len = if unwritten {
                        MAX_UNWRITTEN_EXTENT_LEN
                    } else {
                        MAX_INITIALIZED_EXTENT_LEN
                    };
                    let data = update
                        .allocations
                        .allocate((hole.end - current).min(u32::from(max_len)))?;
                    if !unwritten {
                        zero_new_blocks(fs, &data)?;
                    }
                    let extent = Extent::new(current, (data.end - data.start) as u16, data.start)
                        .with_unwritten(unwritten);
                    let mut node = (*path.levels.last().unwrap().node).clone();
                    let NodeEntries::Leaf(entries) = &mut node.entries else {
                        unreachable!()
                    };
                    let at = entries.partition_point(|e| e.block() < extent.block());
                    entries.insert(at, extent);
                    merge_adjacent_extents(entries);
                    self.finish_leaf_update(&path, &mut update, node)?;
                    let update_end =
                        current
                            .checked_add(u32::from(extent.len()))
                            .ok_or_else(|| {
                                Error::with_message(Errno::EINVAL, "extent logical range overflow")
                            })?;
                    update.invalidated = current..update_end;
                    update.account(self.raw_block_ptrs.sector_count)?;
                    self.accept_update(update)?;
                    current = update_end;
                }
            }
        }
        Ok(())
    }

    /// Stages changed nodes, propagating only new indexes or changed first keys.
    fn finish_leaf_update(
        &self,
        path: &ExtentPath,
        update: &mut PreparedUpdate<'_>,
        mut node: ExtentNode,
    ) -> Result<()> {
        let mut carry = None;
        for (level_index, level) in path.levels.iter().enumerate().rev() {
            if node.depth != level.node.depth {
                unreachable!();
            }
            if let Some(bid) = level.bid {
                if node.len() > node.capacity {
                    let right_bid = update.metadata_block()?;
                    let middle = node.len() / 2;
                    let right_entries = match &mut node.entries {
                        NodeEntries::Leaf(e) => NodeEntries::Leaf(e.split_off(middle)),
                        NodeEntries::Index(e) => NodeEntries::Index(e.split_off(middle)),
                    };
                    let right = ExtentNode {
                        depth: node.depth,
                        capacity: EXTERNAL_NODE_CAPACITY,
                        entries: right_entries,
                    };
                    debug_assert!(node.len() <= node.capacity);
                    debug_assert!(right.len() <= right.capacity);
                    carry = Some(ExtentIdx::new(right.first_key().unwrap(), right_bid));
                    update.nodes.insert(right_bid, Arc::new(right));
                }
                let key = node.first_key().unwrap();
                update.nodes.insert(bid, Arc::new(node));
                let parent = &path.levels[level_index - 1];
                let NodeEntries::Index(entries) = &parent.node.entries else {
                    unreachable!()
                };
                let key_changed = entries[parent.slot].block() != key;
                if !key_changed && carry.is_none() {
                    break;
                }
                node = (*parent.node).clone();
                let NodeEntries::Index(entries) = &mut node.entries else {
                    unreachable!()
                };
                if key_changed {
                    entries[parent.slot] = ExtentIdx::new(key, bid);
                }
                if let Some(index) = carry.take() {
                    entries.insert(parent.slot + 1, index);
                }
            } else {
                if node.len() > node.capacity {
                    if node.depth == MAX_EXTENT_DEPTH {
                        return_errno_with_message!(Errno::EFBIG, "extent tree depth limit reached");
                    }
                    let bid = update.metadata_block()?;
                    let key = node.first_key().unwrap();
                    node.capacity = EXTERNAL_NODE_CAPACITY;
                    let depth = node.depth + 1;
                    update.nodes.insert(bid, Arc::new(node));
                    node = ExtentNode {
                        depth,
                        capacity: INLINE_NODE_CAPACITY,
                        entries: NodeEntries::Index(vec![ExtentIdx::new(key, bid)]),
                    };
                }
                update.root = node;
                break;
            }
        }
        Ok(())
    }

    /// Publishes only initialized intervals after their data I/O has completed.
    pub(in block_mapping) fn mark_written(
        &mut self,
        fs: &Ext4,
        start: usize,
        end: usize,
    ) -> Result<()> {
        if start > end || end as u128 > u128::from(Iblock::MAX) {
            return_errno_with_message!(Errno::EINVAL, "invalid logical conversion range");
        }
        let mut current = start as u64;
        while current < end as u64 {
            let path = self.lookup_path(fs, current as Iblock)?;
            let mut node = (*path.levels.last().unwrap().node).clone();
            let NodeEntries::Leaf(entries) = &mut node.entries else {
                unreachable!()
            };
            let Some(at) = entries.iter().position(|e| e.covers(current as Iblock)) else {
                return_errno_with_message!(
                    Errno::EIO,
                    "unmapped range during unwritten conversion"
                );
            };
            let extent = entries[at];
            let stop = extent.logical_end().min(end as u64);
            if extent.is_unwritten() {
                let mut replacement = Vec::new();
                if u64::from(extent.block()) < current {
                    replacement.push(extent.slice(extent.block(), current));
                }
                replacement.push(extent.slice(current as Iblock, stop).with_unwritten(false));
                if stop < extent.logical_end() {
                    replacement.push(extent.slice(stop as Iblock, extent.logical_end()));
                }
                entries.splice(at..at + 1, replacement);
                merge_adjacent_extents(entries);
                let mut update = PreparedUpdate::new(
                    fs,
                    (*path.levels[0].node).clone(),
                    self.raw_block_ptrs.sector_count,
                );
                self.finish_leaf_update(&path, &mut update, node)?;
                update.invalidated = current as Iblock..stop as Iblock;
                update.account(self.raw_block_ptrs.sector_count)?;
                self.accept_update(update)?;
            }
            current = stop;
        }
        Ok(())
    }

    /// Removes only the suffix subtrees intersecting the new end.
    /// Preparation errors leave the mapping unchanged; release errors after
    /// acceptance leave the new mapping dirty and the remaining frees queued.
    pub(in block_mapping) fn truncate_to_byte_len(
        &mut self,
        fs: &Ext4,
        new_size: usize,
    ) -> Result<()> {
        let update = self.prepare_truncate(fs, new_size)?;
        self.commit_truncate(update)
    }

    /// Publishes a validated candidate; only retired-block cleanup can fail.
    pub(in block_mapping) fn commit_truncate(
        &mut self,
        update: Option<PreparedUpdate<'_>>,
    ) -> Result<()> {
        match update {
            Some(update) => self.accept_validated_update(update),
            None => Ok(()),
        }
    }

    /// Builds and validates a candidate without changing mappings or freeing blocks.
    pub(in block_mapping) fn prepare_truncate<'a>(
        &self,
        fs: &'a Ext4,
        new_size: usize,
    ) -> Result<Option<PreparedUpdate<'a>>> {
        let keep = new_size.div_ceil(BLOCK_SIZE) as u64;
        if keep >= u64::from(Iblock::MAX) {
            return Ok(None);
        }
        let root = self.root_node(fs)?;
        root.validate_bounds(0, u64::from(Iblock::MAX), true)?;
        let mut update = PreparedUpdate::new(fs, (*root).clone(), self.raw_block_ptrs.sector_count);
        update.invalidated = keep as Iblock..Iblock::MAX;
        let result = self.prune_node(
            fs,
            &root,
            0..u64::from(Iblock::MAX),
            true,
            keep,
            &mut update,
        )?;
        update.root = match result {
            PruneResult::Unchanged => return Ok(None),
            PruneResult::Removed => ExtentNode {
                depth: 0,
                capacity: INLINE_NODE_CAPACITY,
                entries: NodeEntries::Leaf(Vec::new()),
            },
            PruneResult::Replaced(root) => root,
        };
        while let NodeEntries::Index(entries) = &update.root.entries {
            if entries.len() != 1 {
                break;
            }
            let bid = entries[0].leaf();
            let child = match update.nodes.get(&bid) {
                Some(node) => node.clone(),
                None => self.node_cache.load(fs, bid, update.root.depth - 1)?,
            };
            if child.len() > INLINE_NODE_CAPACITY {
                break;
            }
            update.root = (*child).clone();
            update.root.capacity = INLINE_NODE_CAPACITY;
            update.retire_node(bid);
        }
        update.account(self.raw_block_ptrs.sector_count)?;
        self.validate_update(&update)?;
        Ok(Some(update))
    }

    fn prune_node(
        &self,
        fs: &Ext4,
        node: &ExtentNode,
        bounds: Range<u64>,
        root: bool,
        keep: u64,
        update: &mut PreparedUpdate<'_>,
    ) -> Result<PruneResult> {
        node.validate_bounds(bounds.start, bounds.end, root)?;
        // No candidate is needed for an unchanged node or a fully retired one.
        let entries = match &node.entries {
            NodeEntries::Leaf(entries) => {
                let first_removed = entries.partition_point(|e| e.logical_end() <= keep);
                if first_removed == entries.len() {
                    return Ok(PruneResult::Unchanged);
                }
                let boundary_kept = u64::from(entries[first_removed].block()) < keep;
                let mut kept = Vec::with_capacity(first_removed + usize::from(boundary_kept));
                kept.extend_from_slice(&entries[..first_removed]);
                for &e in &entries[first_removed..] {
                    let len = keep
                        .saturating_sub(u64::from(e.block()))
                        .min(u64::from(e.len())) as u16;
                    if len != 0 {
                        kept.push(e.slice(e.block(), u64::from(e.block()) + u64::from(len)));
                    }
                    update
                        .retired
                        .push(e.start() + u32::from(len)..e.start() + u32::from(e.len()));
                }
                NodeEntries::Leaf(kept)
            }
            NodeEntries::Index(entries) => {
                // Only the child spanning the boundary and its right siblings
                // can change. Retained subtrees are never loaded.
                let first = entries
                    .partition_point(|e| u64::from(e.block()) <= keep)
                    .saturating_sub(1);
                let mut kept: Option<Vec<ExtentIdx>> = None;
                for (batch_index, indexes) in entries[first..].chunks(READ_BATCH_NODES).enumerate()
                {
                    let children = self.node_cache.load_children(fs, indexes, node.depth - 1)?;
                    for (offset, (&e, child)) in indexes.iter().zip(children).enumerate() {
                        let i = first + batch_index * READ_BATCH_NODES + offset;
                        let end = entries
                            .get(i + 1)
                            .map_or(bounds.end, |e| u64::from(e.block()));
                        let result = self.prune_node(
                            fs,
                            &child,
                            u64::from(e.block())..end,
                            false,
                            keep,
                            update,
                        )?;
                        let child = match result {
                            PruneResult::Unchanged => {
                                if let Some(kept) = &mut kept {
                                    kept.push(e);
                                }
                                continue;
                            }
                            PruneResult::Removed => None,
                            PruneResult::Replaced(child) => Some(child),
                        };
                        let kept = kept.get_or_insert_with(|| {
                            let mut kept = Vec::with_capacity(i + usize::from(child.is_some()));
                            kept.extend_from_slice(&entries[..i]);
                            kept
                        });
                        if let Some(child) = child {
                            kept.push(ExtentIdx::new(child.first_key().unwrap(), e.leaf()));
                            update.nodes.insert(e.leaf(), Arc::new(child));
                        } else {
                            update.retire_node(e.leaf());
                        }
                    }
                }
                let Some(kept) = kept else {
                    return Ok(PruneResult::Unchanged);
                };
                NodeEntries::Index(kept)
            }
        };
        let empty = match &entries {
            NodeEntries::Leaf(entries) => entries.is_empty(),
            NodeEntries::Index(entries) => entries.is_empty(),
        };
        Ok(if empty {
            PruneResult::Removed
        } else {
            PruneResult::Replaced(ExtentNode {
                depth: node.depth,
                capacity: node.capacity,
                entries,
            })
        })
    }

    fn validate_update(&self, update: &PreparedUpdate<'_>) -> Result<()> {
        update.root.validate_addresses(update.allocations.fs)?;
        for node in update.nodes.values() {
            node.validate_addresses(update.allocations.fs)?;
        }
        Ok(())
    }

    fn accept_update(&mut self, update: PreparedUpdate<'_>) -> Result<()> {
        self.validate_update(&update)?;
        self.accept_validated_update(update)
    }

    fn accept_validated_update(&mut self, mut update: PreparedUpdate<'_>) -> Result<()> {
        *self.root_cache.get_mut() = Some(Arc::new(update.root.clone()));
        // Splits and merges preserve mappings outside the logical edit span.
        self.lookup_cache
            .lock()
            .invalidate(update.invalidated.clone());
        for bid in &update.removed {
            self.node_cache.remove(*bid);
        }
        for (bid, node) in core::mem::take(&mut update.nodes) {
            self.node_cache.stage(bid, node);
        }
        let mut root = [0u32; super::RAW_BLOCK_PTRS_LEN];
        update.root.write_into(root.as_mut_bytes());
        // Leaf-only edits need not dirty inode metadata when neither the root
        // nor sector accounting changes.
        if self.raw_block_ptrs.block_ptrs != root
            || self.raw_block_ptrs.sector_count != update.sectors
        {
            *self.raw_block_ptrs = RawBlockPtrs::new(update.sectors, root);
        }
        // All new blocks now belong to this mapping. Drop must not release them.
        update.allocations.ranges.clear();
        DeferredReclaim::release(update.allocations.fs, core::mem::take(&mut update.retired))
    }
}

pub(super) const ZERO_BATCH_BLOCKS: usize = 256;

/// Coalesces compatible adjacent entries without replacing their buffer.
fn merge_adjacent_extents(entries: &mut Vec<Extent>) {
    entries.dedup_by(|next, previous| {
        let combined_len = u32::from(previous.len()) + u32::from(next.len());
        let max_len = if next.is_unwritten() {
            MAX_UNWRITTEN_EXTENT_LEN
        } else {
            MAX_INITIALIZED_EXTENT_LEN
        };
        if previous.logical_end() == u64::from(next.block())
            && previous.start() + u32::from(previous.len()) == next.start()
            && previous.is_unwritten() == next.is_unwritten()
            && combined_len <= u32::from(max_len)
        {
            *previous = Extent::new(previous.block(), combined_len as u16, previous.start())
                .with_unwritten(previous.is_unwritten());
            true
        } else {
            false
        }
    });
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

impl<'a> AllocatedBlocks<'a> {
    fn new(fs: &'a Ext4) -> Self {
        Self {
            fs,
            ranges: Vec::new(),
        }
    }
    fn allocate(&mut self, count: u32) -> Result<Range<Ext4Bid>> {
        let range = self.fs.alloc_blocks(count, 0)?;
        self.ranges.push(range.clone());
        Ok(range)
    }
}

impl Drop for AllocatedBlocks<'_> {
    fn drop(&mut self) {
        let _ = DeferredReclaim::release(self.fs, core::mem::take(&mut self.ranges));
    }
}

impl<'a> PreparedUpdate<'a> {
    fn new(fs: &'a Ext4, root: ExtentNode, sectors: u32) -> Self {
        Self {
            root,
            nodes: BTreeMap::new(),
            removed: Vec::new(),
            retired: Vec::new(),
            allocations: AllocatedBlocks::new(fs),
            sectors,
            invalidated: 0..Iblock::MAX,
        }
    }

    fn metadata_block(&mut self) -> Result<Ext4Bid> {
        Ok(self.allocations.allocate(1)?.start)
    }

    fn retire_node(&mut self, bid: Ext4Bid) {
        self.nodes.remove(&bid);
        self.removed.push(bid);
        self.retired.push(bid..bid + 1);
    }

    fn account(&mut self, old: u32) -> Result<()> {
        let added: u64 = self
            .allocations
            .ranges
            .iter()
            .map(|r| u64::from(r.end - r.start))
            .sum();
        let removed: u64 = self
            .retired
            .iter()
            .map(|r| u64::from(r.end - r.start))
            .sum();
        let sectors = i128::from(old)
            + (i128::from(added) - i128::from(removed)) * i128::from(SECTORS_PER_BLOCK);
        self.sectors = u32::try_from(sectors).map_err(|_| {
            Error::with_message(Errno::EOVERFLOW, "extent sector accounting overflow")
        })?;
        Ok(())
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::*;
    use crate::fs::ext4::{fs::MountFlavor, test_utils::ExtFixtureBuilder};

    #[ktest]
    fn extent_merge_preserves_buffer_and_length_boundaries() {
        for unwritten in [false, true] {
            let limit = if unwritten { 32767u16 } else { 32768u16 };
            let end = u32::from(limit);
            let mut entries = Vec::with_capacity(8);
            for (logical, len, physical, state) in [
                (0, 10, 100, unwritten),
                (10, 10, 110, unwritten),
                (20, limit - 20, 120, unwritten),
                (end, 1, 100 + end, unwritten),
                (end + 1, 1, 101 + end, !unwritten),
                (end + 2, 1, 200000, !unwritten),
                (end + 4, 1, 200001, !unwritten),
            ] {
                entries.push(Extent::new(logical, len, physical).with_unwritten(state));
            }
            let buffer = entries.as_ptr();
            let capacity = entries.capacity();
            merge_adjacent_extents(&mut entries);
            let actual: Vec<_> = entries
                .iter()
                .map(|e| (e.block(), e.len(), e.start(), e.is_unwritten()))
                .collect();
            assert_eq!(
                actual,
                vec![
                    (0, limit, 100, unwritten),
                    (end, 1, 100 + end, unwritten),
                    (end + 1, 1, 101 + end, !unwritten),
                    (end + 2, 1, 200000, !unwritten),
                    (end + 4, 1, 200001, !unwritten),
                ]
            );
            assert_eq!(entries.as_ptr(), buffer);
            assert_eq!(entries.capacity(), capacity);
        }
    }

    #[ktest]
    fn rejected_allocation_drop_transfers_failed_cleanup_to_filesystem() {
        let f = ExtFixtureBuilder::new(1, 2048)
            .with_mount_flavor(MountFlavor::Ext4)
            .with_free_blocks(64, 64)
            .build()
            .unwrap();
        let mut owned = AllocatedBlocks::new(&f.fs);
        owned.allocate(1).unwrap();
        let free = f.fs.super_block().read_state().free_blocks_count();
        f.fs.super_block()
            .write_state()
            .set_free_blocks_count(u32::MAX);
        drop(owned);
        f.fs.super_block().write_state().set_free_blocks_count(free);
        f.fs.retry_deferred_reclaim().unwrap();
        assert_eq!(f.fs.super_block().read_state().free_blocks_count(), 64);
        f.fs.retry_deferred_reclaim().unwrap();
        assert_eq!(f.fs.super_block().read_state().free_blocks_count(), 64);
    }
}
