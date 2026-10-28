// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(inode, block_mapping)]

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
    pub(super) upper: Iblock,
}

/// A mapped logical interval or a proven hole, clipped to the query bounds.
/// Mapped intervals include both initialized and unwritten extents.
#[derive(Clone, Debug)]
pub(in inode) enum ExtentMappingRange {
    Mapped {
        logical: Range<Iblock>,
        extent: Extent,
    },
    Hole(Range<Iblock>),
}

impl ExtentTree {
    pub(in block_mapping) fn next_io_range(
        &self,
        fs: &Ext4,
        range: &mut Range<Iblock>,
        cursor: &mut ExtentRangeCursor,
    ) -> Result<crate::fs::ext4::inode::io_range::IoRange> {
        use crate::fs::ext4::inode::io_range::{Backing, IoRange};
        let start = range.start;
        match cursor.next_range(self, fs, range.clone())? {
            ExtentMappingRange::Mapped { logical, extent } => {
                range.start = logical.end;
                let bid = extent.physical_block(start);
                Ok(IoRange {
                    logical,
                    backing: if extent.is_unwritten() {
                        Backing::Unwritten {
                            physical_start: bid,
                        }
                    } else {
                        Backing::Written {
                            physical_start: bid,
                        }
                    },
                })
            }
            ExtentMappingRange::Hole(hole) => {
                range.start = hole.end;
                Ok(IoRange {
                    logical: hole,
                    backing: Backing::Hole,
                })
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
        )?);
        node.validate_addresses(fs)?;
        *self.root_cache.lock() = Some(node.clone());
        Ok(node)
    }

    pub(super) fn lookup_path(&self, fs: &Ext4, iblock: Iblock) -> Result<ExtentPath> {
        let mut node = self.root_node(fs)?;
        let mut bid = None;
        let mut lower = 0;
        let mut upper = Iblock::MAX;
        let mut levels = Vec::new();
        loop {
            node.validate_bounds(u64::from(lower), u64::from(upper), bid.is_none())?;
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
                    lower = entries[slot].block();
                    let parent_upper = upper;
                    upper = entries.get(slot + 1).map_or(upper, |e| e.block());
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

    pub(super) fn lookup_mapping_range(
        &self,
        fs: &Ext4,
        iblock: Iblock,
        end: Iblock,
    ) -> Result<ExtentMappingRange> {
        if end <= iblock {
            return_errno_with_message!(Errno::EINVAL, "invalid extent query range");
        }
        let path = self.lookup_path(fs, iblock)?;
        Ok(leaf_mapping_range(path.levels.last().unwrap(), iblock, end))
    }
}

/// A traversal path owned by an I/O iterator under its mapping read guard.
/// Cached nodes cannot replace this path: consecutive ranges must not restart
/// root traversal even when all device reads happen to hit the node cache.
#[derive(Debug, Default)]
pub(in inode) struct ExtentRangeCursor {
    path: Option<ExtentPath>,
}

impl ExtentRangeCursor {
    /// Returns the mapped or hole interval beginning at the requested start.
    ///
    /// Consecutive calls advance the retained path under the mapping lock.
    /// Calls must advance monotonically.
    fn next_range(
        &mut self,
        tree: &ExtentTree,
        fs: &Ext4,
        range: Range<Iblock>,
    ) -> Result<ExtentMappingRange> {
        if range.is_empty() {
            return_errno_with_message!(Errno::EINVAL, "invalid extent query range");
        }
        if self.path.is_none() {
            self.path = Some(tree.lookup_path(fs, range.start)?);
        }
        while range.start >= self.path.as_ref().unwrap().levels.last().unwrap().upper {
            // Advance publishes the new suffix only after every load succeeds.
            self.advance(tree, fs)?;
        }
        let leaf = self.path.as_ref().unwrap().levels.last().unwrap();
        let mapping_range = leaf_mapping_range(leaf, range.start, range.end);
        Ok(mapping_range)
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
            let mut lower = entry.block();
            let mut upper = entries.get(slot + 1).map_or(parent.upper, |e| e.block());
            let mut bid = entry.leaf();
            let mut depth = parent.node.depth - 1;
            let mut suffix = Vec::new();
            loop {
                let node = tree.node_cache.load(fs, bid, depth)?;
                node.validate_bounds(u64::from(lower), u64::from(upper), false)?;
                let next = match &node.entries {
                    NodeEntries::Leaf(_) => None,
                    NodeEntries::Index(entries) => Some((
                        entries[0].leaf(),
                        entries[0].block(),
                        entries.get(1).map_or(upper, |e| e.block()),
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

fn leaf_mapping_range(leaf: &PathLevel, iblock: Iblock, end: Iblock) -> ExtentMappingRange {
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
        return ExtentMappingRange::Mapped {
            logical: iblock..end.min(e.logical_end() as Iblock),
            extent: e,
        };
    }
    let stop = entries.get(next).map_or(leaf.upper, |e| e.block());
    ExtentMappingRange::Hole(iblock..end.min(stop))
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::{super::RawBlockPtrs, *};
    use crate::fs::ext4::{fs::MountFlavor, test_utils::ExtFixtureBuilder};

    #[ktest]
    fn extent_cursor_reuses_path_within_leaf_and_across_sibling() {
        let f = ExtFixtureBuilder::new(1, 2048)
            .with_mount_flavor(MountFlavor::Ext4)
            .build()
            .unwrap();
        for (bid, extents) in [
            (40, vec![Extent::new(0, 1, 50), Extent::new(2, 1, 51)]),
            (42, vec![Extent::new(100, 1, 52)]),
            (44, vec![Extent::new(200, 1, 53)]),
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
            entries: NodeEntries::Index(vec![
                ExtentIdx::new(0, 40),
                ExtentIdx::new(100, 42),
                ExtentIdx::new(200, 44),
            ]),
        };
        let mut raw = [0u32; 15];
        root.write_into(raw.as_mut_bytes());
        let tree = ExtentTree::new(RawBlockPtrs::new(48, raw)).unwrap();
        let mut cursor = ExtentRangeCursor::default();
        cursor.next_range(&tree, &f.fs, 0..102).unwrap();
        let root_handle = cursor.path.as_ref().unwrap().levels[0].node.clone();
        let another_path = tree.lookup_path(&f.fs, 2).unwrap();
        assert!(Arc::ptr_eq(&root_handle, &another_path.levels[0].node));

        assert!(
            matches!(cursor.next_range(&tree, &f.fs, 1..102).unwrap(),ExtentMappingRange::Hole(r) if r == (1..2))
        );
        cursor.next_range(&tree, &f.fs, 2..102).unwrap();
        assert!(
            matches!(cursor.next_range(&tree, &f.fs, 3..102).unwrap(),ExtentMappingRange::Hole(r) if r == (3..100))
        );
        // A cached sibling must not leave an established cursor on the old leaf.
        tree.lookup_mapping_range(&f.fs, 100, 102).unwrap();
        cursor.next_range(&tree, &f.fs, 100..102).unwrap();
        assert_eq!(
            cursor.path.as_ref().unwrap().levels.last().unwrap().bid,
            Some(42)
        );
        assert!(Arc::ptr_eq(
            &root_handle,
            &cursor.path.as_ref().unwrap().levels[0].node
        ));
        assert_eq!(f.disk.block_reads(40), 1);
        assert_eq!(f.disk.block_reads(42), 1);
        // A miss after the cached sibling advances from that sibling. Failed
        // child I/O must leave the old path usable for a successful retry.
        assert!(matches!(cursor.next_range(&tree, &f.fs, 101..202).unwrap(),
            ExtentMappingRange::Hole(r) if r == (101..200)));
        f.disk.fail_read_block(Some(44));
        assert!(cursor.next_range(&tree, &f.fs, 200..202).is_err());
        assert_eq!(
            cursor.path.as_ref().unwrap().levels.last().unwrap().bid,
            Some(42)
        );
        assert_eq!(f.disk.block_reads(40), 1);
        assert_eq!(f.disk.block_reads(42), 1);
        f.disk.fail_read_block(None);
        assert!(matches!(cursor.next_range(&tree, &f.fs, 200..202).unwrap(),
            ExtentMappingRange::Mapped { logical, extent }
                if logical == (200..201) && extent.physical_block(200) == 53));
        assert_eq!(
            cursor.path.as_ref().unwrap().levels.last().unwrap().bid,
            Some(44)
        );
        assert!(Arc::ptr_eq(
            &root_handle,
            &cursor.path.as_ref().unwrap().levels[0].node
        ));
        assert_eq!(f.disk.block_reads(40), 1);
        assert_eq!(f.disk.block_reads(42), 1);
        assert_eq!(f.disk.block_reads(44), 2);
        assert!(cursor.next_range(&tree, &f.fs, 202..202).is_err());
        assert!(
            cursor
                .next_range(
                    &tree,
                    &f.fs,
                    Range {
                        start: 203,
                        end: 202
                    }
                )
                .is_err()
        );
    }
}
