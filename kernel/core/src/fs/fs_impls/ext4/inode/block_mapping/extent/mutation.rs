// SPDX-License-Identifier: MPL-2.0

use ostd::mm::io::util::HasVmReaderWriter;

use super::{
    AllocationMode, ExtentTree, RawBlockPtrs, SECTORS_PER_BLOCK,
    node::{ENTRY_SIZE, Extent, ExtentIdx, ExtentNode, MAX_EXTENT_DEPTH, NodeEntries},
    path::{ExtentPath, ExtentRun},
};
use crate::fs::fs_impls::ext4::{fs::Ext4, prelude::*, reclaim::DeferredReclaim};

/// A candidate mapping owns every new allocation until it is accepted.
/// Dropping a rejected candidate releases memory-resident allocation bits;
/// failed releases transfer ownership to the filesystem retry queue.
struct PreparedUpdate<'a> {
    root: ExtentNode,
    nodes: BTreeMap<Ext4Bid, Arc<ExtentNode>>,
    removed: Vec<Ext4Bid>,
    retired: Vec<Range<Ext4Bid>>,
    allocations: AllocatedBlocks<'a>,
    sectors: u32,
}

/// Owns new blocks until the prepared mapping accepts them.
struct AllocatedBlocks<'a> {
    fs: &'a Ext4,
    ranges: Vec<Range<Ext4Bid>>,
}

impl ExtentTree {
    /// Fills only proven holes. Each accepted run is retained if a later run fails.
    pub(in crate::fs::fs_impls::ext4::inode::block_mapping) fn allocate_range_blocks(
        &mut self,
        fs: &Ext4,
        start: usize,
        end: usize,
    ) -> Result<()> {
        self.allocate_range(fs, start, end, AllocationMode::Initialized)
    }

    pub(in crate::fs::fs_impls::ext4::inode::block_mapping) fn allocate_range(
        &mut self,
        fs: &Ext4,
        start: usize,
        end: usize,
        mode: AllocationMode,
    ) -> Result<()> {
        let unwritten = mode == AllocationMode::Unwritten;
        if end as u128 > 1u128 << 32 || start > end {
            return_errno_with_message!(Errno::EINVAL, "invalid logical allocation range");
        }
        let mut current = start as u64;
        while current < end as u64 {
            match self.lookup_run(fs, current as u32, end as u64)? {
                ExtentRun::Mapped { logical, extent } => {
                    if !unwritten && extent.is_unwritten() {
                        let bid = extent.physical_block(current as Iblock);
                        zero_new_blocks(fs, &(bid..bid + (logical.end - current) as u32))?;
                        self.mark_written(fs, current as usize, logical.end as usize)?;
                    }
                    current = logical.end;
                }
                ExtentRun::Hole(hole) => {
                    let path = self.lookup_path(fs, current as u32)?;
                    let mut update = PreparedUpdate::new(
                        fs,
                        (*path.levels[0].node).clone(),
                        self.raw_block_ptrs.sector_count,
                    );
                    let data =
                        update
                            .allocations
                            .allocate((hole.end - current).min(if unwritten {
                                32767
                            } else {
                                32768
                            }) as u32)?;
                    if !unwritten {
                        zero_new_blocks(fs, &data)?;
                    }
                    let extent =
                        Extent::new(current as u32, (data.end - data.start) as u16, data.start)
                            .with_unwritten(unwritten);
                    let mut node = (*path.levels.last().unwrap().node).clone();
                    let NodeEntries::Leaf(entries) = &mut node.entries else {
                        unreachable!()
                    };
                    let at = entries.partition_point(|e| e.block() < extent.block());
                    entries.insert(at, extent);
                    *entries = merge_adjacent_extents(core::mem::take(entries));
                    self.finish_leaf_update(&path, &mut update, node)?;
                    update.account(self.raw_block_ptrs.sector_count)?;
                    self.accept_update(update)?;
                    current += u64::from(extent.len());
                }
            }
        }
        Ok(())
    }

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
                    let mut right = node.clone();
                    let middle = node.len() / 2;
                    right.entries = match &mut node.entries {
                        NodeEntries::Leaf(e) => NodeEntries::Leaf(e.split_off(middle)),
                        NodeEntries::Index(e) => NodeEntries::Index(e.split_off(middle)),
                    };
                    carry = Some(ExtentIdx::new(right.first_key().unwrap(), right_bid));
                    update.nodes.insert(right_bid, Arc::new(right));
                }
                let key = node.first_key().unwrap();
                update.nodes.insert(bid, Arc::new(node));
                let parent = &path.levels[level_index - 1];
                node = (*parent.node).clone();
                let NodeEntries::Index(entries) = &mut node.entries else {
                    unreachable!()
                };
                entries[parent.slot] = ExtentIdx::new(key, bid);
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
                    node.capacity = (BLOCK_SIZE - ENTRY_SIZE) / ENTRY_SIZE;
                    let depth = node.depth + 1;
                    update.nodes.insert(bid, Arc::new(node));
                    node = ExtentNode {
                        depth,
                        capacity: 4,
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
    pub(in crate::fs::fs_impls::ext4::inode::block_mapping) fn mark_written(
        &mut self,
        fs: &Ext4,
        start: usize,
        end: usize,
    ) -> Result<()> {
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
                *entries = merge_adjacent_extents(core::mem::take(entries));
                let mut update = PreparedUpdate::new(
                    fs,
                    (*path.levels[0].node).clone(),
                    self.raw_block_ptrs.sector_count,
                );
                self.finish_leaf_update(&path, &mut update, node)?;
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
    #[cfg(ktest)]
    pub(super) fn truncate_to_byte_len(&mut self, fs: &Ext4, new_size: usize) -> Result<()> {
        self.truncate_with_status(fs, new_size)?.into_result()
    }

    pub(in crate::fs::fs_impls::ext4::inode::block_mapping) fn truncate_with_status(
        &mut self,
        fs: &Ext4,
        new_size: usize,
    ) -> Result<super::super::TruncateCompletion> {
        let keep = new_size.div_ceil(BLOCK_SIZE) as u64;
        if keep >= 1u64 << 32 {
            return Ok(super::super::TruncateCompletion { cleanup: Ok(()) });
        }
        let root = self.root_node(fs)?;
        root.validate_bounds(0, 1u64 << 32, true)?;
        let mut update = PreparedUpdate::new(fs, (*root).clone(), self.raw_block_ptrs.sector_count);
        let (new_root, changed) =
            self.prune_node(fs, &root, 0..1u64 << 32, true, keep, &mut update)?;
        if !changed {
            return Ok(super::super::TruncateCompletion { cleanup: Ok(()) });
        }
        update.root = new_root.unwrap_or(ExtentNode {
            depth: 0,
            capacity: 4,
            entries: NodeEntries::Leaf(Vec::new()),
        });
        while let NodeEntries::Index(entries) = &update.root.entries {
            if entries.len() != 1 {
                break;
            }
            let bid = entries[0].leaf();
            let child = match update.nodes.get(&bid) {
                Some(node) => node.clone(),
                None => self.node_cache.load(fs, bid, update.root.depth - 1)?,
            };
            if child.len() > 4 {
                break;
            }
            update.root = (*child).clone();
            update.root.capacity = 4;
            update.retire_node(bid);
        }
        update.account(self.raw_block_ptrs.sector_count)?;
        Ok(super::super::TruncateCompletion {
            cleanup: self.accept_update(update),
        })
    }

    fn prune_node(
        &self,
        fs: &Ext4,
        node: &ExtentNode,
        bounds: Range<u64>,
        root: bool,
        keep: u64,
        update: &mut PreparedUpdate<'_>,
    ) -> Result<(Option<ExtentNode>, bool)> {
        node.validate_bounds(bounds.start, bounds.end, root)?;
        let mut result = node.clone();
        let mut changed = false;
        match &node.entries {
            NodeEntries::Leaf(entries) => {
                let mut kept = Vec::new();
                for &e in entries {
                    if e.logical_end() <= keep {
                        kept.push(e);
                        continue;
                    }
                    changed = true;
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
                result.entries = NodeEntries::Leaf(kept);
            }
            NodeEntries::Index(entries) => {
                let mut kept = Vec::new();
                for (i, &e) in entries.iter().enumerate() {
                    let end = entries
                        .get(i + 1)
                        .map_or(bounds.end, |e| u64::from(e.block()));
                    if end <= keep {
                        kept.push(e);
                        continue;
                    }
                    let child = self.node_cache.load(fs, e.leaf(), node.depth - 1)?;
                    let (child, edited) = self.prune_node(
                        fs,
                        &child,
                        u64::from(e.block())..end,
                        false,
                        keep,
                        update,
                    )?;
                    changed |= edited;
                    if let Some(child) = child {
                        kept.push(ExtentIdx::new(child.first_key().unwrap(), e.leaf()));
                        if edited {
                            update.nodes.insert(e.leaf(), Arc::new(child));
                        }
                    } else {
                        update.retire_node(e.leaf());
                        changed = true;
                    }
                }
                result.entries = NodeEntries::Index(kept);
            }
        }
        Ok((
            if result.len() == 0 {
                None
            } else {
                Some(result)
            },
            changed,
        ))
    }

    fn accept_update(&mut self, mut update: PreparedUpdate<'_>) -> Result<()> {
        update.root.validate_addresses(update.allocations.fs)?;
        for node in update.nodes.values() {
            node.validate_addresses(update.allocations.fs)?;
        }
        *self.root_cache.get_mut() = Some(Arc::new(update.root.clone()));
        for bid in &update.removed {
            self.node_cache.remove(*bid);
        }
        for (bid, node) in core::mem::take(&mut update.nodes) {
            self.node_cache.stage(bid, node);
        }
        let mut root = [0u32; super::RAW_BLOCK_PTRS_LEN];
        update.root.write_into(root.as_mut_bytes());
        *self.raw_block_ptrs = RawBlockPtrs::new(update.sectors, root);
        // All new blocks now belong to this mapping. Drop must not release them.
        update.allocations.ranges.clear();
        DeferredReclaim::release(update.allocations.fs, core::mem::take(&mut update.retired))
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::*;
    use crate::fs::ext4::{fs::MountFlavor, test_utils::Ext4FixtureBuilder};

    #[ktest]
    fn rejected_allocation_drop_transfers_failed_cleanup_to_filesystem() {
        let f = Ext4FixtureBuilder::new(1, 2048)
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
        f.fs.retry_extent_work().unwrap();
        assert_eq!(f.fs.super_block().read_state().free_blocks_count(), 64);
        f.fs.retry_extent_work().unwrap();
        assert_eq!(f.fs.super_block().read_state().free_blocks_count(), 64);
    }
}

pub(super) const ZERO_BATCH_BLOCKS: usize = 256;

fn merge_adjacent_extents(extents: Vec<Extent>) -> Vec<Extent> {
    let mut merged: Vec<Extent> = Vec::with_capacity(extents.len());
    for extent in extents {
        let Some(previous) = merged.last_mut() else {
            merged.push(extent);
            continue;
        };
        let combined_len = u32::from(previous.len()) + u32::from(extent.len());
        if previous.logical_end() == u64::from(extent.block())
            && previous.start() + u32::from(previous.len()) == extent.start()
            && previous.is_unwritten() == extent.is_unwritten()
            && combined_len <= if extent.is_unwritten() { 32767 } else { 32768 }
        {
            *previous = Extent::new(previous.block(), combined_len as u16, previous.start())
                .with_unwritten(previous.is_unwritten());
        } else {
            merged.push(extent);
        }
    }
    merged
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
