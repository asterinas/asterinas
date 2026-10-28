// SPDX-License-Identifier: MPL-2.0

#[cfg(ktest)]
use super::node::ExtentIdx;
use super::{
    ExtentTree,
    node::{Extent, ExtentNode, NodeEntries},
};
use crate::fs::fs_impls::ext4::{fs::Ext4, prelude::*};

/// One operation's path, retained under the owning mapping lock.
#[derive(Debug)]
pub(super) struct ExtentPath {
    pub(super) levels: Vec<PathLevel>,
}

/// A selected node and child slot, with the exclusive parent-imposed upper bound.
/// A missing block ID identifies the inode-resident root.
#[derive(Debug)]
pub(super) struct PathLevel {
    pub(super) bid: Option<Ext4Bid>,
    pub(super) node: Arc<ExtentNode>,
    pub(super) slot: usize,
    pub(super) upper: u64,
}

/// A mapped interval or a proven hole, bounded by the caller's requested end.
#[derive(Clone, Debug)]
pub(in crate::fs::fs_impls::ext4::inode) enum ExtentRun {
    Mapped { logical: Range<u64>, extent: Extent },
    Hole(Range<u64>),
}

impl ExtentTree {
    pub(in crate::fs::fs_impls::ext4::inode::block_mapping) fn next_io_range(
        &self,
        fs: &Ext4,
        range: &mut Range<Iblock>,
        cursor: &mut ExtentRangeCursor,
    ) -> Result<crate::fs::ext4::inode::io_range::IoRange> {
        use crate::fs::ext4::inode::io_range::IoRange;
        let start = range.start;
        match cursor.run(self, fs, start, u64::from(range.end))? {
            ExtentRun::Mapped { logical, extent } => {
                range.start = logical.end as Iblock;
                let bid = extent.physical_block(start);
                let physical = bid..bid + (logical.end - logical.start) as u32;
                Ok(if extent.is_unwritten() {
                    IoRange::Unwritten(physical)
                } else {
                    IoRange::Mapped(physical)
                })
            }
            ExtentRun::Hole(hole) => {
                range.start = hole.end as Iblock;
                Ok(IoRange::Hole(start..range.start))
            }
        }
    }
}

impl ExtentTree {
    pub(super) fn root_node(&self, fs: &Ext4) -> Result<Arc<ExtentNode>> {
        if let Some(node) = self.root_cache.lock().as_ref() {
            return Ok(node.clone());
        }
        let node = Arc::new(ExtentNode::parse(
            self.raw_block_ptrs.block_ptrs.as_bytes(),
            None,
        )?);
        node.validate_addresses(fs)?;
        *self.root_cache.lock() = Some(node.clone());
        Ok(node)
    }

    pub(super) fn lookup_path(&self, fs: &Ext4, iblock: Iblock) -> Result<ExtentPath> {
        let mut node = self.root_node(fs)?;
        let mut bid = None;
        let mut lower = 0;
        let mut upper = 1u64 << 32;
        let mut levels = Vec::new();
        loop {
            node.validate_bounds(lower, upper, bid.is_none())?;
            match &node.entries {
                NodeEntries::Leaf(entries) => {
                    let slot = entries
                        .partition_point(|e| e.block() <= iblock)
                        .saturating_sub(1);
                    levels.push(PathLevel {
                        bid,
                        node,
                        slot,
                        upper,
                    });
                    return Ok(ExtentPath { levels });
                }
                NodeEntries::Index(entries) => {
                    let slot = entries
                        .partition_point(|e| e.block() <= iblock)
                        .saturating_sub(1);
                    let child_bid = entries[slot].leaf();
                    lower = u64::from(entries[slot].block());
                    let parent_upper = upper;
                    upper = entries
                        .get(slot + 1)
                        .map_or(upper, |e| u64::from(e.block()));
                    let expected = node.depth - 1;
                    levels.push(PathLevel {
                        bid,
                        node,
                        slot,
                        upper: parent_upper,
                    });
                    node = self.node_cache.load(fs, child_bid, expected)?;
                    bid = Some(child_bid);
                }
            }
        }
    }

    pub(super) fn lookup_run(&self, fs: &Ext4, iblock: Iblock, end: u64) -> Result<ExtentRun> {
        if end <= u64::from(iblock) || end > 1u64 << 32 {
            return_errno_with_message!(Errno::EINVAL, "invalid extent query range");
        }
        if let Some(run) = self.cached_run(iblock, end) {
            return Ok(run);
        }
        let run = self.lookup_run_uncached(fs, iblock, end)?;
        self.remember_run(&run);
        Ok(run)
    }

    fn cached_run(&self, iblock: Iblock, end: u64) -> Option<ExtentRun> {
        self.lookup_cache.lock().lookup(iblock, end)
    }

    fn remember_run(&self, run: &ExtentRun) {
        self.lookup_cache.lock().record(run.clone());
    }

    fn lookup_run_uncached(&self, fs: &Ext4, iblock: Iblock, end: u64) -> Result<ExtentRun> {
        if end <= u64::from(iblock) || end > 1u64 << 32 {
            return_errno_with_message!(Errno::EINVAL, "invalid extent query range");
        }
        let path = self.lookup_path(fs, iblock)?;
        Ok(leaf_run(path.levels.last().unwrap(), iblock, end))
    }
}

/// A traversal path owned by an I/O iterator under its mapping read guard.
/// Cached nodes cannot replace this path: consecutive runs must not restart
/// root traversal even when all device reads happen to hit the node cache.
#[derive(Debug, Default)]
pub(in crate::fs::fs_impls::ext4::inode) struct ExtentRangeCursor {
    path: Option<ExtentPath>,
}

impl ExtentRangeCursor {
    fn run(&mut self, tree: &ExtentTree, fs: &Ext4, start: Iblock, end: u64) -> Result<ExtentRun> {
        if let Some(run) = tree.cached_run(start, end) {
            return Ok(run);
        }
        let run = self.run_uncached(tree, fs, start, end)?;
        tree.remember_run(&run);
        Ok(run)
    }

    fn run_uncached(
        &mut self,
        tree: &ExtentTree,
        fs: &Ext4,
        start: Iblock,
        end: u64,
    ) -> Result<ExtentRun> {
        if self.path.is_none() {
            self.path = Some(tree.lookup_path(fs, start)?);
        }
        let path = self.path.as_mut().unwrap();
        if u64::from(start) >= path.levels.last().unwrap().upper {
            self.advance(tree, fs)?;
            // Advance can fail without publishing a partial path.
            return self.run(tree, fs, start, end);
        }
        let leaf = path.levels.last().unwrap();
        Ok(leaf_run(leaf, start, end))
    }

    fn advance(&mut self, tree: &ExtentTree, fs: &Ext4) -> Result<()> {
        let path = self.path.as_ref().unwrap();
        for level in (0..path.levels.len() - 1).rev() {
            let parent = &path.levels[level];
            let NodeEntries::Index(entries) = &parent.node.entries else {
                unreachable!()
            };
            let slot = parent.slot + 1;
            let Some(entry) = entries.get(slot) else {
                continue;
            };
            let mut lower = u64::from(entry.block());
            let mut upper = entries
                .get(slot + 1)
                .map_or(parent.upper, |e| u64::from(e.block()));
            let mut bid = entry.leaf();
            let mut depth = parent.node.depth - 1;
            let mut suffix = Vec::new();
            loop {
                let node = tree.node_cache.load(fs, bid, depth)?;
                node.validate_bounds(lower, upper, false)?;
                let next = match &node.entries {
                    NodeEntries::Leaf(_) => None,
                    NodeEntries::Index(entries) => Some((
                        entries[0].leaf(),
                        u64::from(entries[0].block()),
                        entries.get(1).map_or(upper, |e| u64::from(e.block())),
                    )),
                };
                suffix.push(PathLevel {
                    bid: Some(bid),
                    node,
                    slot: 0,
                    upper,
                });
                let Some((next_bid, next_lower, next_upper)) = next else {
                    break;
                };
                bid = next_bid;
                lower = next_lower;
                upper = next_upper;
                depth -= 1;
            }
            let path = self.path.as_mut().unwrap();
            path.levels.truncate(level + 1);
            path.levels[level].slot = slot;
            path.levels.extend(suffix);
            return Ok(());
        }
        return_errno_with_message!(
            Errno::EUCLEAN,
            "extent cursor has no successor before boundary"
        );
    }
}

fn leaf_run(leaf: &PathLevel, iblock: Iblock, end: u64) -> ExtentRun {
    let NodeEntries::Leaf(entries) = &leaf.node.entries else {
        unreachable!()
    };
    let next = entries.partition_point(|e| e.block() <= iblock);
    if let Some(e) = next
        .checked_sub(1)
        .and_then(|i| entries.get(i))
        .copied()
        .filter(|e| e.covers(iblock))
    {
        return ExtentRun::Mapped {
            logical: u64::from(iblock)..end.min(e.logical_end()),
            extent: e,
        };
    }
    let stop = entries
        .get(next)
        .map_or(leaf.upper, |e| u64::from(e.block()));
    ExtentRun::Hole(u64::from(iblock)..end.min(stop))
}

impl ExtentRun {
    pub(super) fn bounds(&self) -> &Range<u64> {
        match self {
            Self::Mapped { logical, .. } => logical,
            Self::Hole(range) => range,
        }
    }

    pub(super) fn clip(mut self, start: u64, end: u64) -> Self {
        let range = match &mut self {
            Self::Mapped { logical, .. } => logical,
            Self::Hole(range) => range,
        };
        range.start = start.max(range.start);
        range.end = end.min(range.end);
        self
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::{super::RawBlockPtrs, *};
    use crate::fs::ext4::{fs::MountFlavor, test_utils::Ext4FixtureBuilder};

    #[ktest]
    fn extent_cursor_reuses_path_within_leaf_and_across_sibling() {
        let f = Ext4FixtureBuilder::new(1, 2048)
            .with_mount_flavor(MountFlavor::Ext4)
            .build()
            .unwrap();
        for (bid, extents) in [
            (40, vec![Extent::new(0, 1, 50), Extent::new(2, 1, 51)]),
            (42, vec![Extent::new(100, 1, 52)]),
        ] {
            let node = ExtentNode {
                depth: 0,
                capacity: 340,
                entries: NodeEntries::Leaf(extents),
            };
            let mut bytes = [0u8; BLOCK_SIZE];
            node.write_into(&mut bytes);
            f.disk
                .segment()
                .write_bytes(Bid::new(bid).to_offset(), &bytes)
                .unwrap();
        }
        let root = ExtentNode {
            depth: 1,
            capacity: 4,
            entries: NodeEntries::Index(vec![ExtentIdx::new(0, 40), ExtentIdx::new(100, 42)]),
        };
        let mut raw = [0u32; 15];
        root.write_into(raw.as_mut_bytes());
        let mut tree = ExtentTree::new(RawBlockPtrs::new(40, raw)).unwrap();
        let mut cursor = ExtentRangeCursor::default();
        cursor.run(&tree, &f.fs, 0, 102).unwrap();
        let root_handle = cursor.path.as_ref().unwrap().levels[0].node.clone();
        let another_path = tree.lookup_path(&f.fs, 2).unwrap();
        assert!(Arc::ptr_eq(&root_handle, &another_path.levels[0].node));

        assert!(matches!(cursor.run(&tree,&f.fs,1,102).unwrap(),ExtentRun::Hole(r) if r == (1..2)));
        cursor.run(&tree, &f.fs, 2, 102).unwrap();
        assert!(
            matches!(cursor.run(&tree,&f.fs,3,102).unwrap(),ExtentRun::Hole(r) if r == (3..100))
        );
        cursor.run(&tree, &f.fs, 100, 102).unwrap();
        assert!(Arc::ptr_eq(
            &root_handle,
            &cursor.path.as_ref().unwrap().levels[0].node
        ));
        assert_eq!(f.disk.block_reads(40), 1);
        assert_eq!(f.disk.block_reads(42), 1);
        // Separate data-I/O calls must retain proven mapped and hole facts,
        // even after the underlying clean node has been evicted.
        drop(cursor);
        tree.node_cache = super::super::cache::ExtentNodeCache::new();
        let mut range = 0..3;
        let mut cursor = ExtentRangeCursor::default();
        tree.next_io_range(&f.fs, &mut range, &mut cursor).unwrap();
        tree.next_io_range(&f.fs, &mut range, &mut cursor).unwrap();
        tree.node_cache = super::super::cache::ExtentNodeCache::new();
        f.disk.fail_read_block(Some(40));
        for (start, stop) in [(0, 1), (1, 2)] {
            let mut range = start..stop;
            tree.next_io_range(&f.fs, &mut range, &mut ExtentRangeCursor::default())
                .unwrap();
            assert_eq!(range.start, stop);
        }
    }
}
