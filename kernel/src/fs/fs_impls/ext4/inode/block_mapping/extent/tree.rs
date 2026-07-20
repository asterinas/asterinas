// SPDX-License-Identifier: MPL-2.0

//! Extent-tree lookup: maps a logical block to its leaf extent.
//!
//! The tree root lives inline in the inode's 60-byte `i_block`. Interior nodes
//! hold index entries pointing to child blocks read from the device; leaf nodes
//! hold the extents that map logical blocks to physical runs.

use super::{
    super::super::super::{fs::Ext4, prelude::*, utils},
    node::{
        EXTENT_MAGIC, Extent, ExtentHeader, ExtentIdx, ExtentKind, MAX_WRITTEN_LEN, RawExtent,
        RawExtentHeader, RawExtentIdx,
    },
};

/// Size of one extent-tree entry (header, index, or leaf), in bytes.
const ENTRY_SIZE: usize = 12;

/// Maximum extent-tree depth, mirroring `EXT4_MAX_EXTENT_DEPTH`.
const MAX_DEPTH: u32 = 5;

/// A validated extent tree rooted in an inode's inline `i_block` field.
pub(super) struct ExtentTree {
    root: [u32; super::super::super::RAW_BLOCK_PTRS_LEN],
}

impl ExtentTree {
    /// Parses and validates an inline extent-tree root.
    pub(super) fn try_from_root(
        root: [u32; super::super::super::RAW_BLOCK_PTRS_LEN],
    ) -> Result<Self> {
        ExtentHeader::try_from(&RawExtentHeader::from_bytes(
            &root.as_bytes()[0..ENTRY_SIZE],
        ))?;
        Ok(Self { root })
    }

    /// Returns the serialized inline root for inode writeback.
    pub(super) const fn root(&self) -> [u32; super::super::super::RAW_BLOCK_PTRS_LEN] {
        self.root
    }

    pub(super) fn find(&self, device: &dyn BlockDevice, iblock: Iblock) -> Result<Option<Extent>> {
        find_extent(&self.root, device, iblock)
    }

    pub(super) fn insert(
        &mut self,
        fs: &Ext4,
        iblock: Iblock,
        pblock: Ext4Bid,
        len: u16,
        kind: ExtentKind,
    ) -> Result<TreeDelta> {
        insert_extent(&mut self.root, fs, iblock, pblock, len, kind)
    }

    pub(super) fn extents(&self, device: &dyn BlockDevice) -> Result<Vec<Extent>> {
        flatten_extents(&self.root, device)
    }

    pub(super) fn convert_unwritten(
        &mut self,
        fs: &Ext4,
        iblock: Iblock,
        len: u32,
    ) -> Result<TreeDelta> {
        convert_unwritten(&mut self.root, fs, iblock, len)
    }

    pub(super) fn external_leaf_count(&self, device: &dyn BlockDevice) -> Result<u32> {
        external_leaf_count(&self.root, device)
    }

    pub(super) fn rebuild(
        &mut self,
        fs: &Ext4,
        extents: &[(Iblock, u16, Ext4Bid, ExtentKind)],
    ) -> Result<u32> {
        rebuild_from_extents(&mut self.root, fs, extents)
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

/// Searches one node's bytes for the entry covering `iblock`.
///
/// Entries are sorted by logical block, so the covering entry is the last one
/// whose starting block is `<= iblock`.
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
            let extent = Extent::try_from(&RawExtent::from_bytes(&bytes[off..off + ENTRY_SIZE]))?;
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

/// Walks the extent tree rooted in `root` (the inode's `i_block`) to find the
/// extent covering `iblock`, returning `None` for a hole.
fn find_extent(
    root: &[u32; super::super::super::RAW_BLOCK_PTRS_LEN],
    device: &dyn BlockDevice,
    iblock: Iblock,
) -> Result<Option<Extent>> {
    let mut next_bid = match search_node(root.as_bytes(), iblock)? {
        Step::Found(extent) => return Ok(Some(extent)),
        Step::Hole => return Ok(None),
        Step::Descend(bid) => bid,
    };

    for _ in 0..MAX_DEPTH {
        let block =
            device.read_val::<[u8; BLOCK_SIZE]>(utils::block_offset(next_bid, BLOCK_SIZE)?)?;
        match search_node(&block, iblock)? {
            Step::Found(extent) => return Ok(Some(extent)),
            Step::Hole => return Ok(None),
            Step::Descend(bid) => next_bid = bid,
        }
    }
    return_errno_with_message!(Errno::EUCLEAN, "extent tree deeper than maximum depth");
}

/// Maximum extents in the inline (depth-0) root: the 60-byte `i_block` holds a
/// 12-byte header plus four 12-byte entries.
const INLINE_MAX: usize = 4;

/// Maximum extents in one full-block external leaf node.
const LEAF_MAX: usize = (BLOCK_SIZE - ENTRY_SIZE) / ENTRY_SIZE;

/// The metadata (index/leaf) blocks a tree mutation allocated and freed, so the
/// caller can keep the inode's `i_blocks` accounting correct.
pub(super) struct TreeDelta {
    pub(super) meta_allocated: u32,
    pub(super) meta_freed: u32,
}

/// Inserts the extent mapping `[iblock, iblock+len)` → `[pblock, pblock+len)`
/// into the tree rooted in the inode's `i_block`, rebuilding the on-disk layout.
///
/// The tree is flattened to a sorted extent list, the new run is merged in,
/// and the list is re-serialized as an inline (≤ [`INLINE_MAX`] extents) or
/// depth-1 tree. The caller must guarantee `[iblock, iblock + len)` is a hole.
///
/// External leaf blocks are written synchronously and reused when possible.
fn insert_extent(
    root: &mut [u32; super::super::super::RAW_BLOCK_PTRS_LEN],
    fs: &Ext4,
    iblock: Iblock,
    pblock: Ext4Bid,
    len: u16,
    kind: ExtentKind,
) -> Result<TreeDelta> {
    let device = fs.block_device().as_ref();
    let (mut extents, old_external) = flatten(root, device)?;
    extents.push(Extent::new(iblock, len, pblock, kind));
    merge_extents(&mut extents);
    reserialize(root, fs, &extents, &old_external)
}

/// Parses the whole extent tree into a list of leaf extents sorted by logical
/// block. Used by the write path to plan hole runs from a tree snapshot.
fn flatten_extents(
    root: &[u32; super::super::super::RAW_BLOCK_PTRS_LEN],
    device: &dyn BlockDevice,
) -> Result<Vec<Extent>> {
    let (mut extents, _external) = flatten(root, device)?;
    extents.sort_by_key(|e| e.block());
    Ok(extents)
}

/// Returns how many external (depth-1) leaf blocks the tree currently owns, so
/// the truncate path can compute the exact metadata-block delta for `i_blocks`.
fn external_leaf_count(
    root: &[u32; super::super::super::RAW_BLOCK_PTRS_LEN],
    device: &dyn BlockDevice,
) -> Result<u32> {
    let (_extents, external) = flatten(root, device)?;
    u32::try_from(external.len())
        .map_err(|_| Error::with_message(Errno::EOVERFLOW, "too many extent leaf blocks"))
}

/// Rebuilds the on-disk extent tree from the exact set of `extents` to keep
/// (each `(iblock, len, pblock, kind)`), reusing/freeing external leaf blocks as
/// needed, and returns the number of external leaf blocks the rebuilt tree
/// references. Used by the truncate path.
fn rebuild_from_extents(
    root: &mut [u32; super::super::super::RAW_BLOCK_PTRS_LEN],
    fs: &Ext4,
    extents: &[(Iblock, u16, Ext4Bid, ExtentKind)],
) -> Result<u32> {
    let device = fs.block_device().as_ref();
    let (_old, old_external) = flatten(root, device)?;
    let extents: Vec<Extent> = extents
        .iter()
        .map(|&(block, len, start, kind)| Extent::new(block, len, start, kind))
        .collect();
    reserialize(root, fs, &extents, &old_external)?;
    // After reserialization, count the external leaves the new root references.
    let (_new, new_external) = flatten(root, device)?;
    u32::try_from(new_external.len())
        .map_err(|_| Error::with_message(Errno::EOVERFLOW, "too many extent leaf blocks"))
}

/// Converts the unwritten parts of the logical range `[iblock, iblock + len)`
/// to written, splitting any overlapping unwritten extent so the written
/// sub-range keeps the same physical mapping. Used by the write path so data
/// written into preallocated (unwritten) extents becomes readable.
///
/// Each overlapping unwritten extent splits into up to three runs — an
/// unwritten head `[e.block, ov_start)`, a written middle `[ov_start, ov_end)`
/// at the same physical offset, and an unwritten tail `[ov_end, e.end)` —
/// dropping empty parts. Written and non-overlapping extents are untouched.
///
/// No data blocks are allocated or freed: the physical mapping is preserved, so
/// `i_blocks` changes only by the net metadata-block delta a split may cause
/// (returned in the [`TreeDelta`]).
fn convert_unwritten(
    root: &mut [u32; super::super::super::RAW_BLOCK_PTRS_LEN],
    fs: &Ext4,
    iblock: Iblock,
    len: u32,
) -> Result<TreeDelta> {
    let device = fs.block_device().as_ref();
    let (extents, old_external) = flatten(root, device)?;

    let range_start = iblock;
    let range_end = u64::from(iblock) + u64::from(len);

    let mut converted: Vec<Extent> = Vec::with_capacity(extents.len() + 2);
    for e in &extents {
        let e_start = e.block();
        let e_end = u64::from(e_start) + u64::from(e.len());
        // Leave written extents and any extent fully outside the range as-is.
        if !e.is_unwritten() || e_end <= u64::from(range_start) || u64::from(e_start) >= range_end {
            converted.push(*e);
            continue;
        }

        let ov_start = e_start.max(range_start);
        let ov_end = e_end.min(range_end);

        // Unwritten head before the overlap.
        if ov_start > e_start {
            converted.push(Extent::new(
                e_start,
                u16::try_from(ov_start - e_start).expect("extent head length fits u16"),
                e.start(),
                ExtentKind::Unwritten,
            ));
        }
        // Written middle: same physical mapping, shifted by the head length.
        let mid_start = e.start() + Ext4Bid::from(ov_start - e_start);
        converted.push(Extent::new(
            ov_start,
            u16::try_from(ov_end - u64::from(ov_start)).expect("extent overlap length fits u16"),
            mid_start,
            ExtentKind::Written,
        ));
        // Unwritten tail after the overlap.
        if ov_end < e_end {
            let tail_block = Iblock::try_from(ov_end)
                .map_err(|_| Error::with_message(Errno::EOVERFLOW, "logical block overflow"))?;
            let tail_start = e.start() + (ov_end - u64::from(e_start));
            converted.push(Extent::new(
                tail_block,
                u16::try_from(e_end - ov_end).expect("extent tail length fits u16"),
                tail_start,
                ExtentKind::Unwritten,
            ));
        }
    }

    merge_extents(&mut converted);
    reserialize(root, fs, &converted, &old_external)
}

/// Parses the whole extent tree into a sorted list of leaf extents, also
/// returning the physical blocks of any external (depth-1) leaf nodes.
///
/// The mutation path only builds depth-0 or depth-1 trees, so deeper trees are
/// rejected rather than rewritten.
fn flatten(
    root: &[u32; super::super::super::RAW_BLOCK_PTRS_LEN],
    device: &dyn BlockDevice,
) -> Result<(Vec<Extent>, Vec<Ext4Bid>)> {
    let root_bytes = root.as_bytes();
    let header = ExtentHeader::try_from(&RawExtentHeader::from_bytes(&root_bytes[0..ENTRY_SIZE]))?;
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
        let block = device.read_val::<[u8; BLOCK_SIZE]>(utils::block_offset(bid, BLOCK_SIZE)?)?;
        let leaf_hdr = ExtentHeader::try_from(&RawExtentHeader::from_bytes(&block[0..ENTRY_SIZE]))?;
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
    root: &mut [u32; super::super::super::RAW_BLOCK_PTRS_LEN],
    fs: &Ext4,
    extents: &[Extent],
    old_external: &[Ext4Bid],
) -> Result<TreeDelta> {
    let device = fs.block_device().as_ref();

    if extents.len() <= INLINE_MAX {
        write_inline_leaf_root(root, extents)?;
        // The root no longer references any external block; free them all.
        let mut meta_freed = 0;
        for &bid in old_external {
            free_meta_block(fs, bid)?;
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
        match alloc_meta_block(fs, goal) {
            Ok(bid) => newly_allocated.push(bid),
            Err(err) => {
                for &bid in &newly_allocated {
                    let _ = free_meta_block(fs, bid);
                }
                return Err(err);
            }
        }
    }
    leaf_bids.extend_from_slice(&newly_allocated);

    // Write each leaf node. On failure, roll back the freshly allocated blocks
    // (the in-memory root is not yet updated, so the old tree stays referenced).
    for (chunk, &leaf_bid) in extents.chunks(LEAF_MAX).zip(leaf_bids.iter()) {
        if let Err(err) = write_leaf_node(device, leaf_bid, chunk) {
            for &bid in &newly_allocated {
                let _ = free_meta_block(fs, bid);
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
    write_index_root(root, &index_entries);

    // Free surplus old external blocks the root no longer references.
    let mut meta_freed = 0;
    for &bid in &old_external[reuse..] {
        free_meta_block(fs, bid)?;
        meta_freed += 1;
    }

    Ok(TreeDelta {
        meta_allocated: u32::try_from(newly_allocated.len())
            .map_err(|_| Error::with_message(Errno::EOVERFLOW, "too many extent leaf blocks"))?,
        meta_freed,
    })
}

/// Allocates one metadata block for an external extent-tree node.
fn alloc_meta_block(fs: &Ext4, goal: Ext4Bid) -> Result<Ext4Bid> {
    let range = fs.alloc_blocks(1, goal)?;
    let bid = range.start;
    Ok(bid)
}

/// Frees one external extent-tree metadata block.
fn free_meta_block(fs: &Ext4, bid: Ext4Bid) -> Result<()> {
    fs.free_blocks(bid, 1)
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
fn write_inline_leaf_root(
    root: &mut [u32; super::super::super::RAW_BLOCK_PTRS_LEN],
    extents: &[Extent],
) -> Result<()> {
    let bytes = root.as_mut_bytes();
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
fn write_index_root(
    root: &mut [u32; super::super::super::RAW_BLOCK_PTRS_LEN],
    entries: &[RawExtentIdx],
) {
    let bytes = root.as_mut_bytes();
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

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    use super::*;
    use crate::fs::fs_impls::ext4::test_utils::Ext4FixtureBuilder;

    /// Writes a depth-0 extent root (header + extents) into a 60-byte `i_block`.
    fn inline_root(extents: &[RawExtent]) -> [u32; super::super::super::super::RAW_BLOCK_PTRS_LEN] {
        let mut block = [0u32; super::super::super::super::RAW_BLOCK_PTRS_LEN];
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
        let root = inline_root(&[RawExtent {
            block: 0,
            len: 4,
            start_hi: 0,
            start_lo: 100,
        }]);

        let mapped = find_extent(&root, device, 2).unwrap().unwrap();
        assert_eq!(mapped.start(), 100);
        assert_eq!(mapped.block(), 0);

        // Block 4 is beyond the extent: a hole.
        assert!(find_extent(&root, device, 4).unwrap().is_none());
    }

    #[ktest]
    fn inline_multiple_extents_lookup() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        let device = f.ext4.block_device().as_ref();
        let root = inline_root(&[
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
        ]);

        // Logical 6 → second extent, physical 300 + (6 - 5) = 301.
        let mapped = find_extent(&root, device, 6).unwrap().unwrap();
        assert_eq!(mapped.start() + (6 - mapped.block()) as u64, 301);

        // Logical 3 falls in the gap between the two extents: a hole.
        assert!(find_extent(&root, device, 3).unwrap().is_none());
    }

    #[ktest]
    fn empty_root_is_all_holes() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        let device = f.ext4.block_device().as_ref();
        let root = inline_root(&[]);
        assert!(find_extent(&root, device, 0).unwrap().is_none());
    }

    /// Writes a depth-1 index root into a 60-byte `i_block`, pointing at a single
    /// external leaf node at physical block `leaf_block`.
    fn index_root(leaf_block: u32) -> [u32; super::super::super::super::RAW_BLOCK_PTRS_LEN] {
        let mut block = [0u32; super::super::super::super::RAW_BLOCK_PTRS_LEN];
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
        let root = index_root(leaf_block);

        // Logical 1 → descend to the leaf → first extent (0..2) → physical 301.
        let m0 = find_extent(&root, device, 1).unwrap().unwrap();
        assert_eq!(m0.start() + (1 - m0.block()) as u64, 301);
        // Logical 6 → second extent (5..8) → physical 401.
        let m1 = find_extent(&root, device, 6).unwrap().unwrap();
        assert_eq!(m1.start() + (6 - m1.block()) as u64, 401);
        // Logical 3 → gap between the leaf's extents → hole.
        assert!(find_extent(&root, device, 3).unwrap().is_none());
        // Logical 100 → beyond all extents → hole.
        assert!(find_extent(&root, device, 100).unwrap().is_none());
    }

    /// Returns the depth and entry count of the inline root header.
    fn root_header(root: &[u32; super::super::super::super::RAW_BLOCK_PTRS_LEN]) -> (u16, u16) {
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
        let mut root = inline_root(&[]);

        // [0,2) -> 100, then contiguous [2,2) -> 102 must coalesce into [0,4).
        let d0 = insert_extent(&mut root, &f.ext4, 0, 100, 2, ExtentKind::Written).unwrap();
        assert_eq!((d0.meta_allocated, d0.meta_freed), (0, 0));
        insert_extent(&mut root, &f.ext4, 2, 102, 2, ExtentKind::Written).unwrap();

        // Still inline depth-0 with a single merged extent.
        assert_eq!(root_header(&root), (0, 1));
        let device = f.ext4.block_device().as_ref();
        let m = find_extent(&root, device, 3).unwrap().unwrap();
        assert_eq!(m.start() + (3 - m.block()) as u64, 103);
    }

    #[ktest]
    fn insert_non_contiguous_stays_separate() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048)
            .with_block_bitmap_metadata_marked()
            .build()
            .unwrap();
        let mut root = inline_root(&[]);

        insert_extent(&mut root, &f.ext4, 0, 100, 1, ExtentKind::Written).unwrap();
        insert_extent(&mut root, &f.ext4, 5, 200, 1, ExtentKind::Written).unwrap();

        assert_eq!(root_header(&root), (0, 2));
        let device = f.ext4.block_device().as_ref();
        assert_eq!(find_extent(&root, device, 0).unwrap().unwrap().start(), 100);
        assert_eq!(find_extent(&root, device, 5).unwrap().unwrap().start(), 200);
        assert!(find_extent(&root, device, 3).unwrap().is_none());
    }

    #[ktest]
    fn inline_overflow_grows_to_depth1() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048)
            .with_block_bitmap_metadata_marked()
            .build()
            .unwrap();
        let mut root = inline_root(&[]);

        // Five non-contiguous extents overflow the 4-entry inline root.
        let mut total_allocated = 0;
        for k in 0..5u32 {
            let d = insert_extent(
                &mut root,
                &f.ext4,
                k * 2,
                100 + k as u64 * 10,
                1,
                ExtentKind::Written,
            )
            .unwrap();
            total_allocated += d.meta_allocated;
        }

        // The root is now a depth-1 index with one external leaf.
        assert_eq!(root_header(&root), (1, 1));
        assert_eq!(total_allocated, 1); // exactly one leaf block allocated

        // All five mappings are still reachable through the external leaf.
        let device = f.ext4.block_device().as_ref();
        for k in 0..5u32 {
            let m = find_extent(&root, device, k * 2).unwrap().unwrap();
            assert_eq!(m.start(), 100 + k as u64 * 10);
        }

        // The allocated leaf block is marked in the block bitmap (e2fsck-clean).
        let leaf_bid = ExtentIdx::from(&RawExtentIdx::from_bytes(
            &root.as_bytes()[ENTRY_SIZE..2 * ENTRY_SIZE],
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
        let mut root = inline_root(&[]);
        for k in 0..5u32 {
            insert_extent(
                &mut root,
                &f.ext4,
                k * 2,
                100 + k as u64 * 10,
                1,
                ExtentKind::Written,
            )
            .unwrap();
        }
        assert_eq!(root_header(&root).0, 1);

        // A sixth extent fits the existing leaf: no new metadata block.
        let d = insert_extent(&mut root, &f.ext4, 20, 500, 1, ExtentKind::Written).unwrap();
        assert_eq!((d.meta_allocated, d.meta_freed), (0, 0));

        let device = f.ext4.block_device().as_ref();
        assert_eq!(
            find_extent(&root, device, 20).unwrap().unwrap().start(),
            500
        );
    }

    /// Regression: two contiguous *unwritten* extents whose lengths sum to
    /// `MAX_WRITTEN_LEN` (32768) must NOT coalesce — an unwritten `ee_len` of
    /// 32768 overflows the bias encoding (`len + MAX_WRITTEN_LEN`), so an
    /// unwritten run caps at `EXT_UNWRITTEN_MAX_LEN = 32767`. Written runs of the
    /// same shape may still merge to 32768.
    #[ktest]
    fn merge_caps_unwritten_below_max_len() {
        let half = MAX_WRITTEN_LEN / 2; // 16384

        let mut unwritten = vec![
            Extent::new(0, half, 100, ExtentKind::Unwritten),
            Extent::new(
                half as Iblock,
                half,
                100 + half as Ext4Bid,
                ExtentKind::Unwritten,
            ),
        ];
        merge_extents(&mut unwritten);
        for e in &unwritten {
            assert!(!e.is_unwritten() || e.len() < MAX_WRITTEN_LEN);
        }

        let mut written = vec![
            Extent::new(0, half, 200, ExtentKind::Written),
            Extent::new(
                half as Iblock,
                half,
                200 + half as Ext4Bid,
                ExtentKind::Written,
            ),
        ];
        merge_extents(&mut written);
        assert_eq!(written.len(), 1);
        assert_eq!(written[0].len(), MAX_WRITTEN_LEN);
    }
}
