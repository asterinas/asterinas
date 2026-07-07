// SPDX-License-Identifier: MPL-2.0

//! [`ExtentTree`] — the validated, read-only extent tree of one inode.
//!
//! The tree root lives inline in the inode's 60-byte `i_block`; interior nodes
//! hold index entries pointing to child blocks read from the device, and leaf
//! nodes hold the extents mapping logical blocks to physical runs. The type
//! owns the root (validated once at construction — the parse-once boundary) and
//! the inode's `i_blocks` sector accounting; every tree operation is a method,
//! so only a constructed (i.e. proven well-formed) tree can be searched.
//! Mirrors ext2's `BlockPtrTree`.

use super::{
    super::{
        super::{fs::Ext4, prelude::*, utils},
        RAW_BLOCK_PTRS_LEN,
    },
    node::{
        EXTENT_MAGIC, Extent, ExtentHeader, ExtentIdx, RawExtent, RawExtentHeader, RawExtentIdx,
    },
};

/// Size of one extent-tree entry (header, index, or leaf), in bytes.
const ENTRY_SIZE: usize = 12;

/// Maximum extent-tree depth, mirroring `EXT4_MAX_EXTENT_DEPTH`.
const MAX_DEPTH: u32 = 5;

/// Maximum extents in the inline (depth-0) root: the 60-byte `i_block` holds a
/// 12-byte header plus four 12-byte entries.
const INLINE_MAX: usize = 4;

/// The validated, read-only extent tree of one inode, plus the `i_blocks`
/// sector accounting it exposes for reads.
///
/// `root` is the inode's 60-byte `i_block` in its on-disk layout, **validated
/// at construction** ([`try_new`](Self::try_new)) and thereafter only read — so
/// methods trust it without re-validating (rule: parse once at the boundary).
/// `sector_count` mirrors the inode's `i_blocks` (data + extent-tree metadata,
/// in 512-byte sectors).
///
/// This struct is the "ExtentTree" lock content at position ③ in the global
/// lock order (report §5.1); [`ExtentManager`](super::ExtentManager) wraps it
/// in the `RwMutex` and delegates.
pub(in crate::fs::fs_impls::ext4::inode) struct ExtentTree {
    root: [u32; RAW_BLOCK_PTRS_LEN],
    sector_count: u64,
}

impl ExtentTree {
    /// Validates `root`'s extent header once and takes ownership of the tree.
    ///
    /// This is the parse boundary: a bad magic / entry count / depth is
    /// rejected here, and every later method call trusts the root.
    pub(super) fn try_new(root: [u32; RAW_BLOCK_PTRS_LEN], sector_count: u64) -> Result<Self> {
        ExtentHeader::try_from(&RawExtentHeader::from_bytes(
            &root.as_bytes()[0..ENTRY_SIZE],
        ))?;
        Ok(Self { root, sector_count })
    }

    /// A valid empty tree: a depth-0 header (magic, 0 entries, max 4) followed
    /// by zeros — what a freshly created regular file or directory carries, so
    /// the extent reader sees a well-formed (empty) tree from the first byte.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(in crate::fs::fs_impls::ext4::inode) const fn empty() -> Self {
        let mut root = [0u32; RAW_BLOCK_PTRS_LEN];
        // Each `i_block` word packs two 16-bit fields, little-endian: word 0 is
        // `eh_magic | eh_entries(=0)`, word 1 is `eh_max(=4) | eh_depth(=0)`.
        root[0] = EXTENT_MAGIC as u32;
        root[1] = INLINE_MAX as u32;
        Self {
            root,
            sector_count: 0,
        }
    }

    /// Returns the inode's `i_blocks` (512-byte sectors) accounting.
    pub(super) const fn sector_count(&self) -> u64 {
        self.sector_count
    }

    /// Returns the root's header, decoded from the trusted
    /// (construction-validated) bytes.
    fn header(&self) -> ExtentHeader {
        ExtentHeader::from_trusted(&RawExtentHeader::from_bytes(
            &self.root.as_bytes()[0..ENTRY_SIZE],
        ))
    }

    /// Walks the tree to find the extent covering `iblock`, returning `None`
    /// for a hole.
    ///
    /// External nodes are read through [`utils::read_metadata_block`], the
    /// single metadata-read funnel — a plain device read on this read-only,
    /// non-journaled mount (the seam a journaling layer would later re-widen).
    pub(super) fn lookup(&self, fs: &Ext4, iblock: Iblock) -> Result<Option<Extent>> {
        let root_bytes = self.root.as_bytes();
        let mut next_bid = match search_entries(&self.header(), root_bytes, iblock)? {
            Step::Found(extent) => return Ok(Some(extent)),
            Step::Hole => return Ok(None),
            Step::Descend(bid) => bid,
        };

        let device = fs.block_device().as_ref();
        for _ in 0..MAX_DEPTH {
            let block = utils::read_metadata_block(device, next_bid)?;
            match search_node(&block, iblock)? {
                Step::Found(extent) => return Ok(Some(extent)),
                Step::Hole => return Ok(None),
                Step::Descend(bid) => next_bid = bid,
            }
        }
        return_errno_with_message!(Errno::EUCLEAN, "extent tree deeper than maximum depth");
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

/// Parses and searches one freshly read (untrusted) node — the parse boundary
/// for device bytes.
fn search_node(bytes: &[u8], iblock: Iblock) -> Result<Step> {
    let header = ExtentHeader::try_from(&RawExtentHeader::from_bytes(&bytes[0..ENTRY_SIZE]))?;
    search_entries(&header, bytes, iblock)
}

/// Searches one node's entries for `iblock`, `header` already decoded.
///
/// Entries are sorted by logical block, so the covering entry is the last one
/// whose starting block is `<= iblock`. Phase 1 scans linearly (nodes hold at
/// most a few hundred entries); a binary search is a later optimization.
fn search_entries(header: &ExtentHeader, bytes: &[u8], iblock: Iblock) -> Result<Step> {
    let nr_entries = header.entries() as usize;

    let entries_end = ENTRY_SIZE * (1 + nr_entries);
    if entries_end > bytes.len() {
        return_errno_with_message!(Errno::EUCLEAN, "extent node entries overrun node");
    }

    if header.is_leaf() {
        let mut covering: Option<Extent> = None;
        for i in 0..nr_entries {
            let off = ENTRY_SIZE * (1 + i);
            let extent = Extent::from(&RawExtent::from_bytes(&bytes[off..off + ENTRY_SIZE]));
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
            // `iblock` lies before the first index entry. Linux descends into
            // the first child anyway and then finds no covering extent; we
            // short-circuit to the same answer. On a well-formed tree the two
            // are equivalent (no leaf under index 0 maps anything below its
            // `first_block`); on a corrupt tree the short-circuit is the safer
            // degradation — a hole read instead of chasing a bogus subtree
            // (P1 review item, judged & documented at P5).
            None => Ok(Step::Hole),
        }
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    use super::*;
    use crate::fs::fs_impls::ext4::test_utils::Ext4FixtureBuilder;

    /// Builds a validated tree over a depth-0 root (header + extents).
    fn inline_tree(extents: &[RawExtent]) -> ExtentTree {
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
        ExtentTree::try_new(block, 0).unwrap()
    }

    #[ktest]
    fn inline_single_extent_lookup() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        // One extent mapping logical 0..4 to physical 100..104.
        let tree = inline_tree(&[RawExtent {
            block: 0,
            len: 4,
            start_hi: 0,
            start_lo: 100,
        }]);

        let mapped = tree.lookup(&f.ext4, 2).unwrap().unwrap();
        assert_eq!(mapped.start(), 100);
        assert_eq!(mapped.block(), 0);

        // Block 4 is beyond the extent: a hole.
        assert!(tree.lookup(&f.ext4, 4).unwrap().is_none());
    }

    #[ktest]
    fn inline_multiple_extents_lookup() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        let tree = inline_tree(&[
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
        let mapped = tree.lookup(&f.ext4, 6).unwrap().unwrap();
        assert_eq!(mapped.start() + (6 - mapped.block()) as u64, 301);

        // Logical 3 falls in the gap between the two extents: a hole.
        assert!(tree.lookup(&f.ext4, 3).unwrap().is_none());
    }

    #[ktest]
    fn empty_root_is_all_holes() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        let tree = ExtentTree::empty();
        assert!(tree.lookup(&f.ext4, 0).unwrap().is_none());
    }

    #[ktest]
    fn rejects_bad_root_magic() {
        let root = [0u32; RAW_BLOCK_PTRS_LEN];
        assert!(ExtentTree::try_new(root, 0).is_err());
    }

    /// Builds a validated tree over a depth-1 index root pointing at a single
    /// external leaf node at physical block `leaf_block`.
    fn index_tree(leaf_block: u32) -> ExtentTree {
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
        ExtentTree::try_new(block, 0).unwrap()
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
        let tree = index_tree(leaf_block);

        // Logical 1 → descend to the leaf → first extent (0..2) → physical 301.
        let m0 = tree.lookup(&f.ext4, 1).unwrap().unwrap();
        assert_eq!(m0.start() + (1 - m0.block()) as u64, 301);
        // Logical 6 → second extent (5..8) → physical 401.
        let m1 = tree.lookup(&f.ext4, 6).unwrap().unwrap();
        assert_eq!(m1.start() + (6 - m1.block()) as u64, 401);
        // Logical 3 → gap between the leaf's extents → hole.
        assert!(tree.lookup(&f.ext4, 3).unwrap().is_none());
        // Logical 100 → beyond all extents → hole.
        assert!(tree.lookup(&f.ext4, 100).unwrap().is_none());
    }
}
