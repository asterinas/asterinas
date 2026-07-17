// SPDX-License-Identifier: MPL-2.0

//! Extent-tree lookup: maps a logical block to its leaf extent.
//!
//! The tree root lives inline in the inode's 60-byte `i_block`. Interior nodes
//! hold index entries pointing to child blocks read from the device; leaf nodes
//! hold the extents that map logical blocks to physical runs.

use super::node::{
    EXTENT_MAGIC, Extent, ExtentHeader, ExtentIdx, ExtentKind, MAX_WRITTEN_LEN, RawExtent,
    RawExtentHeader, RawExtentIdx,
};
use crate::fs::fs_impls::ext4::{fs::Ext4, inode::RAW_BLOCK_PTRS_LEN, prelude::*, utils};

/// Size of one extent-tree entry (header, index, or leaf), in bytes.
const ENTRY_SIZE: usize = 12;

/// Maximum extent-tree depth, mirroring `EXT4_MAX_EXTENT_DEPTH`.
const MAX_DEPTH: u32 = 5;

/// A validated extent tree rooted in an inode's inline `i_block` field.
pub(super) struct ExtentTree {
    root: [u32; RAW_BLOCK_PTRS_LEN],
}

impl ExtentTree {
    /// Parses and validates an inline extent-tree root.
    pub(super) fn try_from_root(root: [u32; RAW_BLOCK_PTRS_LEN]) -> Result<Self> {
        ExtentHeader::try_from(&RawExtentHeader::from_bytes(
            &root.as_bytes()[0..ENTRY_SIZE],
        ))?;
        Ok(Self { root })
    }

    /// Returns the serialized inline root for inode writeback.
    pub(super) const fn root(&self) -> [u32; RAW_BLOCK_PTRS_LEN] {
        self.root
    }

    /// Finds the extent covering `iblock`, or `None` when it is a hole.
    pub(super) fn find(&self, device: &dyn BlockDevice, iblock: Iblock) -> Result<Option<Extent>> {
        let mut next_bid = match Self::search_node(self.root.as_bytes(), iblock)? {
            Step::Found(extent) => return Ok(Some(extent)),
            Step::Hole => return Ok(None),
            Step::Descend(bid) => bid,
        };

        for _ in 0..MAX_DEPTH {
            let block =
                device.read_val::<[u8; BLOCK_SIZE]>(utils::block_offset(next_bid, BLOCK_SIZE)?)?;
            match Self::search_node(&block, iblock)? {
                Step::Found(extent) => return Ok(Some(extent)),
                Step::Hole => return Ok(None),
                Step::Descend(bid) => next_bid = bid,
            }
        }
        return_errno_with_message!(Errno::EUCLEAN, "extent tree deeper than maximum depth");
    }

    /// Inserts a mapping into a hole and updates the serialized tree.
    pub(super) fn insert(
        &mut self,
        fs: &Ext4,
        iblock: Iblock,
        pblock: Ext4Bid,
        len: u16,
        kind: ExtentKind,
    ) -> Result<TreeDelta> {
        let device = fs.block_device().as_ref();
        let (mut extents, old_external) = self.flatten(device)?;
        extents.push(Extent::new(iblock, len, pblock, kind));
        Self::merge_extents(&mut extents);
        self.reserialize(fs, &extents, &old_external)
    }

    /// Returns every leaf extent sorted by logical block.
    pub(super) fn extents(&self, device: &dyn BlockDevice) -> Result<Vec<Extent>> {
        let (mut extents, _external) = self.flatten(device)?;
        extents.sort_by_key(|extent| extent.block());
        Ok(extents)
    }

    /// Converts overlapping unwritten mappings to written mappings.
    pub(super) fn convert_unwritten(
        &mut self,
        fs: &Ext4,
        iblock: Iblock,
        len: u32,
    ) -> Result<TreeDelta> {
        let device = fs.block_device().as_ref();
        let (extents, old_external) = self.flatten(device)?;
        let range_start = iblock;
        let range_end = u64::from(iblock) + u64::from(len);

        let mut converted = Vec::with_capacity(extents.len() + 2);
        for extent in &extents {
            let extent_start = extent.block();
            let extent_end = u64::from(extent_start) + u64::from(extent.len());
            if !extent.is_unwritten()
                || extent_end <= u64::from(range_start)
                || u64::from(extent_start) >= range_end
            {
                converted.push(*extent);
                continue;
            }

            let overlap_start = extent_start.max(range_start);
            let overlap_end = extent_end.min(range_end);
            if overlap_start > extent_start {
                converted.push(Extent::new(
                    extent_start,
                    u16::try_from(overlap_start - extent_start)
                        .expect("extent head length fits u16"),
                    extent.start(),
                    ExtentKind::Unwritten,
                ));
            }

            converted.push(Extent::new(
                overlap_start,
                u16::try_from(overlap_end - u64::from(overlap_start))
                    .expect("extent overlap length fits u16"),
                extent.start() + Ext4Bid::from(overlap_start - extent_start),
                ExtentKind::Written,
            ));

            if overlap_end < extent_end {
                converted.push(Extent::new(
                    Iblock::try_from(overlap_end).map_err(|_| {
                        Error::with_message(Errno::EOVERFLOW, "logical block overflow")
                    })?,
                    u16::try_from(extent_end - overlap_end).expect("extent tail length fits u16"),
                    extent.start() + (overlap_end - u64::from(extent_start)),
                    ExtentKind::Unwritten,
                ));
            }
        }

        Self::merge_extents(&mut converted);
        self.reserialize(fs, &converted, &old_external)
    }

    /// Returns the number of external leaf blocks owned by this tree.
    pub(super) fn external_leaf_count(&self, device: &dyn BlockDevice) -> Result<u32> {
        let (_extents, external) = self.flatten(device)?;
        u32::try_from(external.len())
            .map_err(|_| Error::with_message(Errno::EOVERFLOW, "too many extent leaf blocks"))
    }

    /// Rebuilds the tree from the mappings retained by truncate.
    pub(super) fn rebuild(
        &mut self,
        fs: &Ext4,
        extents: &[(Iblock, u16, Ext4Bid, ExtentKind)],
    ) -> Result<u32> {
        let device = fs.block_device().as_ref();
        let (_old, old_external) = self.flatten(device)?;
        let extents: Vec<Extent> = extents
            .iter()
            .map(|&(block, len, start, kind)| Extent::new(block, len, start, kind))
            .collect();
        self.reserialize(fs, &extents, &old_external)?;

        let (_new, new_external) = self.flatten(device)?;
        u32::try_from(new_external.len())
            .map_err(|_| Error::with_message(Errno::EOVERFLOW, "too many extent leaf blocks"))
    }
}

/// The outcome of searching a single extent-tree node for `iblock`.
enum Step {
    /// A leaf extent that covers `iblock`.
    Found(Extent),
    /// No extent covers `iblock`: a hole.
    Hole,
    /// An interior node points to a child at this physical block.
    Descend(Ext4Bid),
}

impl ExtentTree {
    /// Searches one node's bytes for the entry covering `iblock`.
    ///
    /// Entries are sorted by logical block, so the covering entry is the last
    /// one whose starting block is `<= iblock`.
    fn search_node(bytes: &[u8], iblock: Iblock) -> Result<Step> {
        let header = ExtentHeader::try_from(&RawExtentHeader::from_bytes(&bytes[0..ENTRY_SIZE]))?;
        let nr_entries = usize::from(header.entries());

        let entries_end = ENTRY_SIZE * (1 + nr_entries);
        if entries_end > bytes.len() {
            return_errno_with_message!(Errno::EUCLEAN, "extent node entries overrun node");
        }

        if header.is_leaf() {
            let mut covering: Option<Extent> = None;
            for i in 0..nr_entries {
                let off = ENTRY_SIZE * (1 + i);
                let extent =
                    Extent::try_from(&RawExtent::from_bytes(&bytes[off..off + ENTRY_SIZE]))?;
                if extent.block() <= iblock {
                    covering = Some(extent);
                } else {
                    break;
                }
            }
            match covering {
                Some(extent) if extent.covers(iblock) => Ok(Step::Found(extent)),
                _ => Ok(Step::Hole),
            }
        } else {
            let mut chosen: Option<ExtentIdx> = None;
            for i in 0..nr_entries {
                let off = ENTRY_SIZE * (1 + i);
                let idx = ExtentIdx::from(&RawExtentIdx::from_bytes(&bytes[off..off + ENTRY_SIZE]));
                if idx.block() <= iblock {
                    chosen = Some(idx);
                } else {
                    break;
                }
            }
            match chosen {
                Some(idx) => Ok(Step::Descend(idx.leaf())),
                // `iblock` lies before the first index entry: a hole.
                None => Ok(Step::Hole),
            }
        }
    }
}

/// Maximum extents in the inline (depth-0) root: the 60-byte `i_block` holds a
/// 12-byte header plus four 12-byte entries.
const INLINE_MAX: usize = 4;

/// Maximum extents in one full-block external leaf node.
const LEAF_MAX: usize = (BLOCK_SIZE - ENTRY_SIZE) / ENTRY_SIZE;

/// The metadata (index/leaf) blocks a tree mutation allocated and freed, so the
/// caller can keep the inode's `i_blocks` accounting correct.
pub(super) struct TreeDelta {
    /// Number of extent metadata blocks allocated by the mutation.
    pub(super) meta_allocated: u32,
    /// Number of extent metadata blocks freed by the mutation.
    pub(super) meta_freed: u32,
}

impl ExtentTree {
    /// Parses all leaf extents and returns their external metadata blocks.
    ///
    /// The mutation path only builds depth-0 or depth-1 trees, so deeper trees
    /// are rejected rather than rewritten.
    fn flatten(&self, device: &dyn BlockDevice) -> Result<(Vec<Extent>, Vec<Ext4Bid>)> {
        let root_bytes = self.root.as_bytes();
        let header =
            ExtentHeader::try_from(&RawExtentHeader::from_bytes(&root_bytes[0..ENTRY_SIZE]))?;
        let nr = usize::from(header.entries());

        if header.is_leaf() {
            let mut extents = Vec::with_capacity(nr);
            for i in 0..nr {
                let off = ENTRY_SIZE * (1 + i);
                extents.push(Extent::try_from(&RawExtent::from_bytes(
                    &root_bytes[off..off + ENTRY_SIZE],
                ))?);
            }
            return Ok((extents, Vec::new()));
        }

        if header.depth() != 1 {
            return_errno_with_message!(
                Errno::EUCLEAN,
                "cannot rewrite an extent tree deeper than 1"
            );
        }

        let mut leaf_bids = Vec::with_capacity(nr);
        for i in 0..nr {
            let off = ENTRY_SIZE * (1 + i);
            let idx = ExtentIdx::from(&RawExtentIdx::from_bytes(
                &root_bytes[off..off + ENTRY_SIZE],
            ));
            leaf_bids.push(idx.leaf());
        }

        let mut extents = Vec::new();
        for &bid in &leaf_bids {
            let block =
                device.read_val::<[u8; BLOCK_SIZE]>(utils::block_offset(bid, BLOCK_SIZE)?)?;
            let leaf_hdr =
                ExtentHeader::try_from(&RawExtentHeader::from_bytes(&block[0..ENTRY_SIZE]))?;
            if !leaf_hdr.is_leaf() {
                return_errno_with_message!(Errno::EUCLEAN, "depth-1 child is not a leaf");
            }
            let lnr = usize::from(leaf_hdr.entries());
            if ENTRY_SIZE * (1 + lnr) > block.len() {
                return_errno_with_message!(Errno::EUCLEAN, "extent leaf entries overrun node");
            }
            for j in 0..lnr {
                let off = ENTRY_SIZE * (1 + j);
                extents.push(Extent::try_from(&RawExtent::from_bytes(
                    &block[off..off + ENTRY_SIZE],
                ))?);
            }
        }
        Ok((extents, leaf_bids))
    }

    /// Sorts `extents` by logical block and coalesces runs that are logically and
    /// physically contiguous and share the same written/unwritten state.
    fn merge_extents(extents: &mut Vec<Extent>) {
        extents.sort_by_key(|e| e.block());
        let mut merged: Vec<Extent> = Vec::with_capacity(extents.len());
        for e in extents.iter() {
            if let Some(last) = merged.last() {
                // Unwritten extents cap one below the written limit: the length is
                // bias-encoded as `len + MAX_WRITTEN_LEN`, so an unwritten run of
                // `MAX_WRITTEN_LEN` would overflow the encoded `ee_len` value.
                let max_len = if last.is_unwritten() {
                    u32::from(MAX_WRITTEN_LEN) - 1
                } else {
                    u32::from(MAX_WRITTEN_LEN)
                };
                let contiguous = last.block() as u64 + last.len() as u64 == e.block() as u64
                    && last.start() + last.len() as u64 == e.start()
                    && last.is_unwritten() == e.is_unwritten()
                    && u32::from(last.len()) + u32::from(e.len()) <= max_len;
                if contiguous {
                    *merged.last_mut().unwrap() = Extent::new(
                        last.block(),
                        last.len() + e.len(),
                        last.start(),
                        last.kind(),
                    );
                    continue;
                }
            }
            merged.push(*e);
        }
        *extents = merged;
    }

    /// Re-serializes `extents` into the on-disk tree, reusing the existing external
    /// leaf blocks where possible and allocating/freeing the difference.
    fn reserialize(
        &mut self,
        fs: &Ext4,
        extents: &[Extent],
        old_external: &[Ext4Bid],
    ) -> Result<TreeDelta> {
        let device = fs.block_device().as_ref();

        if extents.len() <= INLINE_MAX {
            self.write_inline_leaf_root(extents)?;
            // The root no longer references any external block; free them all.
            let mut meta_freed = 0;
            for &bid in old_external {
                fs.free_blocks(bid, 1)?;
                meta_freed += 1;
            }
            return Ok(TreeDelta {
                meta_allocated: 0,
                meta_freed,
            });
        }

        let nr_leaves = extents.len().div_ceil(LEAF_MAX);
        if nr_leaves > INLINE_MAX {
            return_errno_with_message!(Errno::ENOSPC, "extent tree would exceed depth 1");
        }

        // Reuse old external blocks; allocate any shortfall (rolling back on error).
        let reuse = nr_leaves.min(old_external.len());
        let mut leaf_bids: Vec<Ext4Bid> = old_external[..reuse].to_vec();
        let mut newly_allocated: Vec<Ext4Bid> = Vec::new();
        let goal = extents.first().map(|e| e.start()).unwrap_or(0);
        for _ in reuse..nr_leaves {
            match fs.alloc_blocks(1, goal) {
                Ok(range) => newly_allocated.push(range.start),
                Err(err) => {
                    for &bid in &newly_allocated {
                        let _ = fs.free_blocks(bid, 1);
                    }
                    return Err(err);
                }
            }
        }
        leaf_bids.extend_from_slice(&newly_allocated);

        // Write each leaf node. On failure, roll back the freshly allocated blocks
        // (the in-memory root is not yet updated, so the old tree stays referenced).
        for (chunk, &leaf_bid) in extents.chunks(LEAF_MAX).zip(leaf_bids.iter()) {
            if let Err(err) = Self::write_leaf_node(device, leaf_bid, chunk) {
                for &bid in &newly_allocated {
                    let _ = fs.free_blocks(bid, 1);
                }
                return Err(err);
            }
        }

        // Commit: rewrite the depth-1 index root (in memory, infallible).
        let index_entries: Vec<RawExtentIdx> = extents
            .chunks(LEAF_MAX)
            .zip(leaf_bids.iter())
            .map(|(chunk, &leaf_bid)| RawExtentIdx::new(chunk[0].block(), leaf_bid))
            .collect::<Result<_>>()?;
        self.write_index_root(&index_entries);

        // Free surplus old external blocks the root no longer references.
        let mut meta_freed = 0;
        for &bid in &old_external[reuse..] {
            fs.free_blocks(bid, 1)?;
            meta_freed += 1;
        }

        Ok(TreeDelta {
            meta_allocated: u32::try_from(newly_allocated.len()).map_err(|_| {
                Error::with_message(Errno::EOVERFLOW, "too many extent leaf blocks")
            })?,
            meta_freed,
        })
    }

    /// Serializes `extents` into a full-block external leaf node at `bid`.
    fn write_leaf_node(device: &dyn BlockDevice, bid: Ext4Bid, extents: &[Extent]) -> Result<()> {
        let mut block = [0u8; BLOCK_SIZE];
        let header = RawExtentHeader {
            magic: EXTENT_MAGIC,
            entries: u16::try_from(extents.len()).expect("leaf extent count fits u16"),
            max: u16::try_from(LEAF_MAX).expect("leaf capacity fits u16"),
            depth: 0,
            generation: 0,
        };
        block[0..ENTRY_SIZE].copy_from_slice(header.as_bytes());
        for (i, ext) in extents.iter().enumerate() {
            let off = ENTRY_SIZE * (1 + i);
            block[off..off + ENTRY_SIZE].copy_from_slice(RawExtent::try_from(ext)?.as_bytes());
        }
        device.write_val(utils::block_offset(bid, BLOCK_SIZE)?, &block)?;
        Ok(())
    }

    /// Writes a depth-0 inline leaf root (header + up to [`INLINE_MAX`] extents)
    /// into the inode's 60-byte `i_block`.
    fn write_inline_leaf_root(&mut self, extents: &[Extent]) -> Result<()> {
        let bytes = self.root.as_mut_bytes();
        bytes.fill(0);
        let header = RawExtentHeader {
            magic: EXTENT_MAGIC,
            entries: u16::try_from(extents.len()).expect("inline extent count fits u16"),
            max: u16::try_from(INLINE_MAX).expect("inline capacity fits u16"),
            depth: 0,
            generation: 0,
        };
        bytes[0..ENTRY_SIZE].copy_from_slice(header.as_bytes());
        for (i, ext) in extents.iter().enumerate() {
            let off = ENTRY_SIZE * (1 + i);
            bytes[off..off + ENTRY_SIZE].copy_from_slice(RawExtent::try_from(ext)?.as_bytes());
        }
        Ok(())
    }

    /// Writes a depth-1 index root (header + one index entry per external leaf) into
    /// the inode's 60-byte `i_block`.
    fn write_index_root(&mut self, entries: &[RawExtentIdx]) {
        let bytes = self.root.as_mut_bytes();
        bytes.fill(0);
        let header = RawExtentHeader {
            magic: EXTENT_MAGIC,
            entries: u16::try_from(entries.len()).expect("inline index count fits u16"),
            max: u16::try_from(INLINE_MAX).expect("inline capacity fits u16"),
            depth: 1,
            generation: 0,
        };
        bytes[0..ENTRY_SIZE].copy_from_slice(header.as_bytes());
        for (i, idx) in entries.iter().enumerate() {
            let off = ENTRY_SIZE * (1 + i);
            bytes[off..off + ENTRY_SIZE].copy_from_slice(idx.as_bytes());
        }
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    use super::*;
    use crate::fs::fs_impls::ext4::test_utils::Ext4FixtureBuilder;

    /// Writes a depth-0 extent root (header + extents) into a 60-byte `i_block`.
    fn inline_root(extents: &[RawExtent]) -> [u32; RAW_BLOCK_PTRS_LEN] {
        let mut block = [0u32; RAW_BLOCK_PTRS_LEN];
        let bytes = block.as_mut_bytes();
        let header = RawExtentHeader {
            magic: EXTENT_MAGIC,
            entries: extents.len() as u16,
            max: 4,
            depth: 0,
            generation: 0,
        };
        bytes[0..ENTRY_SIZE].copy_from_slice(header.as_bytes());
        for (i, extent) in extents.iter().enumerate() {
            let off = ENTRY_SIZE * (1 + i);
            bytes[off..off + ENTRY_SIZE].copy_from_slice(extent.as_bytes());
        }
        block
    }

    #[ktest]
    fn inline_single_extent_lookup() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        let device = f.ext4.block_device().as_ref();
        // One extent mapping logical 0..4 to physical 100..104.
        let tree = ExtentTree::try_from_root(inline_root(&[RawExtent {
            block: 0,
            len: 4,
            start_hi: 1,
            start_lo: 100,
        }]))
        .unwrap();

        let mapped = tree.find(device, 2).unwrap().unwrap();
        assert_eq!(mapped.start(), (1 << 32) | 100);
        assert_eq!(mapped.block(), 0);

        // Block 4 is beyond the extent: a hole.
        assert!(tree.find(device, 4).unwrap().is_none());
    }

    #[ktest]
    fn inline_multiple_extents_lookup() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        let device = f.ext4.block_device().as_ref();
        let tree = ExtentTree::try_from_root(inline_root(&[
            RawExtent {
                block: 0,
                len: 2,
                start_hi: 0,
                start_lo: 200,
            },
            RawExtent {
                block: 5,
                len: 3,
                start_hi: 0,
                start_lo: 300,
            },
        ]))
        .unwrap();

        // Logical 6 → second extent, physical 300 + (6 - 5) = 301.
        let mapped = tree.find(device, 6).unwrap().unwrap();
        assert_eq!(mapped.start() + (6 - mapped.block()) as u64, 301);

        // Logical 3 falls in the gap between the two extents: a hole.
        assert!(tree.find(device, 3).unwrap().is_none());
    }

    #[ktest]
    fn empty_root_is_all_holes() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        let device = f.ext4.block_device().as_ref();
        let tree = ExtentTree::try_from_root(inline_root(&[])).unwrap();
        assert!(tree.find(device, 0).unwrap().is_none());
    }

    /// Writes a depth-1 index root into a 60-byte `i_block`, pointing at a single
    /// external leaf node at physical block `leaf_block`.
    fn index_root(leaf_block: u32) -> [u32; RAW_BLOCK_PTRS_LEN] {
        let mut block = [0u32; RAW_BLOCK_PTRS_LEN];
        let bytes = block.as_mut_bytes();
        let header = RawExtentHeader {
            magic: EXTENT_MAGIC,
            entries: 1,
            max: 4,
            depth: 1,
            generation: 0,
        };
        bytes[0..ENTRY_SIZE].copy_from_slice(header.as_bytes());
        let idx = RawExtentIdx {
            block: 0,
            leaf_lo: leaf_block,
            leaf_hi: 0,
            unused: 0,
        };
        bytes[ENTRY_SIZE..2 * ENTRY_SIZE].copy_from_slice(idx.as_bytes());
        block
    }

    /// Builds a full-block external leaf node (depth 0) from `extents`.
    fn leaf_node(extents: &[RawExtent]) -> [u8; BLOCK_SIZE] {
        let mut block = [0u8; BLOCK_SIZE];
        let header = RawExtentHeader {
            magic: EXTENT_MAGIC,
            entries: extents.len() as u16,
            max: ((BLOCK_SIZE / ENTRY_SIZE) - 1) as u16,
            depth: 0,
            generation: 0,
        };
        block[0..ENTRY_SIZE].copy_from_slice(header.as_bytes());
        for (i, extent) in extents.iter().enumerate() {
            let off = ENTRY_SIZE * (1 + i);
            block[off..off + ENTRY_SIZE].copy_from_slice(extent.as_bytes());
        }
        block
    }

    /// A depth-1 tree (index root → external leaf read from the device) must be
    /// descended into. This exercises the interior-node read path that inline
    /// (depth-0) roots never reach — the real-image counterpart is a fragmented
    /// file whose extents overflow the inline root.
    #[ktest]
    fn descends_into_external_leaf() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        let device = f.ext4.block_device().as_ref();

        let leaf_block = 200u32;
        let leaf = leaf_node(&[
            RawExtent {
                block: 0,
                len: 2,
                start_hi: 0,
                start_lo: 300,
            },
            RawExtent {
                block: 5,
                len: 3,
                start_hi: 0,
                start_lo: 400,
            },
        ]);
        f.write_data_block(leaf_block, &leaf);
        let tree = ExtentTree::try_from_root(index_root(leaf_block)).unwrap();

        // Logical 1 → descend to the leaf → first extent (0..2) → physical 301.
        let m0 = tree.find(device, 1).unwrap().unwrap();
        assert_eq!(m0.start() + (1 - m0.block()) as u64, 301);
        // Logical 6 → second extent (5..8) → physical 401.
        let m1 = tree.find(device, 6).unwrap().unwrap();
        assert_eq!(m1.start() + (6 - m1.block()) as u64, 401);
        // Logical 3 → gap between the leaf's extents → hole.
        assert!(tree.find(device, 3).unwrap().is_none());
        // Logical 100 → beyond all extents → hole.
        assert!(tree.find(device, 100).unwrap().is_none());
    }

    /// Returns the depth and entry count of the inline root header.
    fn root_header(root: &[u32; RAW_BLOCK_PTRS_LEN]) -> (u16, u16) {
        let hdr = ExtentHeader::try_from(&RawExtentHeader::from_bytes(
            &root.as_bytes()[0..ENTRY_SIZE],
        ))
        .unwrap();
        (hdr.depth(), hdr.entries())
    }

    #[ktest]
    fn insert_into_inline_merges_contiguous() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048)
            .with_block_bitmap_metadata_marked()
            .build()
            .unwrap();
        let mut tree = ExtentTree::try_from_root(inline_root(&[])).unwrap();

        // [0,2) -> 100, then contiguous [2,2) -> 102 must coalesce into [0,4).
        let d0 = tree
            .insert(&f.ext4, 0, 100, 2, ExtentKind::Written)
            .unwrap();
        assert_eq!((d0.meta_allocated, d0.meta_freed), (0, 0));
        tree.insert(&f.ext4, 2, 102, 2, ExtentKind::Written)
            .unwrap();

        // Still inline depth-0 with a single merged extent.
        assert_eq!(root_header(&tree.root()), (0, 1));
        let device = f.ext4.block_device().as_ref();
        let m = tree.find(device, 3).unwrap().unwrap();
        assert_eq!(m.start() + (3 - m.block()) as u64, 103);
    }

    #[ktest]
    fn insert_non_contiguous_stays_separate() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048)
            .with_block_bitmap_metadata_marked()
            .build()
            .unwrap();
        let mut tree = ExtentTree::try_from_root(inline_root(&[])).unwrap();

        tree.insert(&f.ext4, 0, 100, 1, ExtentKind::Written)
            .unwrap();
        tree.insert(&f.ext4, 5, 200, 1, ExtentKind::Written)
            .unwrap();

        assert_eq!(root_header(&tree.root()), (0, 2));
        let device = f.ext4.block_device().as_ref();
        assert_eq!(tree.find(device, 0).unwrap().unwrap().start(), 100);
        assert_eq!(tree.find(device, 5).unwrap().unwrap().start(), 200);
        assert!(tree.find(device, 3).unwrap().is_none());
    }

    #[ktest]
    fn inline_overflow_grows_to_depth1() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048)
            .with_block_bitmap_metadata_marked()
            .build()
            .unwrap();
        let mut tree = ExtentTree::try_from_root(inline_root(&[])).unwrap();

        // Five non-contiguous extents overflow the 4-entry inline root.
        let mut total_allocated = 0;
        for k in 0..5u32 {
            let d = tree
                .insert(&f.ext4, k * 2, 100 + k as u64 * 10, 1, ExtentKind::Written)
                .unwrap();
            total_allocated += d.meta_allocated;
        }

        // The root is now a depth-1 index with one external leaf.
        assert_eq!(root_header(&tree.root()), (1, 1));
        assert_eq!(total_allocated, 1); // exactly one leaf block allocated

        // All five mappings are still reachable through the external leaf.
        let device = f.ext4.block_device().as_ref();
        for k in 0..5u32 {
            let m = tree.find(device, k * 2).unwrap().unwrap();
            assert_eq!(m.start(), 100 + k as u64 * 10);
        }

        // The allocated leaf block is marked in the block bitmap (e2fsck-clean).
        let leaf_bid = ExtentIdx::from(&RawExtentIdx::from_bytes(
            &tree.root().as_bytes()[ENTRY_SIZE..2 * ENTRY_SIZE],
        ))
        .leaf();
        let group = f.ext4.block_group(0);
        let metadata = group.metadata();
        assert!(
            metadata
                .block_bitmap
                .is_allocated((leaf_bid - group.first_block()) as u16)
        );
    }

    #[ktest]
    fn insert_into_depth1_reuses_leaf() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048)
            .with_block_bitmap_metadata_marked()
            .build()
            .unwrap();
        let mut tree = ExtentTree::try_from_root(inline_root(&[])).unwrap();
        for k in 0..5u32 {
            tree.insert(&f.ext4, k * 2, 100 + k as u64 * 10, 1, ExtentKind::Written)
                .unwrap();
        }
        assert_eq!(root_header(&tree.root()).0, 1);

        // A sixth extent fits the existing leaf: no new metadata block.
        let d = tree
            .insert(&f.ext4, 20, 500, 1, ExtentKind::Written)
            .unwrap();
        assert_eq!((d.meta_allocated, d.meta_freed), (0, 0));

        let device = f.ext4.block_device().as_ref();
        assert_eq!(tree.find(device, 20).unwrap().unwrap().start(), 500);
    }

    /// Regression: two contiguous *unwritten* extents whose lengths sum to
    /// `MAX_WRITTEN_LEN` (32768) must NOT coalesce — an unwritten `ee_len` of
    /// 32768 overflows the bias encoding (`len + MAX_WRITTEN_LEN`), so an
    /// unwritten run caps at `EXT_UNWRITTEN_MAX_LEN = 32767`. Written runs of the
    /// same shape may still merge to 32768.
    #[ktest]
    fn merge_caps_unwritten_below_max_len() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048)
            .with_block_bitmap_metadata_marked()
            .build()
            .unwrap();
        let device = f.ext4.block_device().as_ref();
        let half = MAX_WRITTEN_LEN / 2; // 16384

        let mut unwritten = ExtentTree::try_from_root(inline_root(&[])).unwrap();
        unwritten
            .insert(&f.ext4, 0, 100, half, ExtentKind::Unwritten)
            .unwrap();
        unwritten
            .insert(
                &f.ext4,
                half as Iblock,
                100 + half as Ext4Bid,
                half,
                ExtentKind::Unwritten,
            )
            .unwrap();
        let unwritten = unwritten.extents(device).unwrap();
        for e in &unwritten {
            assert!(!e.is_unwritten() || e.len() < MAX_WRITTEN_LEN);
        }

        let mut written = ExtentTree::try_from_root(inline_root(&[])).unwrap();
        written
            .insert(&f.ext4, 0, 200, half, ExtentKind::Written)
            .unwrap();
        written
            .insert(
                &f.ext4,
                half as Iblock,
                200 + half as Ext4Bid,
                half,
                ExtentKind::Written,
            )
            .unwrap();
        let written = written.extents(device).unwrap();
        assert_eq!(written.len(), 1);
        assert_eq!(written[0].len(), MAX_WRITTEN_LEN);
    }
}
