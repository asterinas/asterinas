// SPDX-License-Identifier: MPL-2.0

//! Ext4 extent block-mapping engine — the inode's logical→physical block
//! translation for reads.
//!
//! This replaces ext2's indirect-block tree. The authoritative state is one
//! [`ExtentTree`] per inode (the validated tree root + `i_blocks` accounting,
//! defined in [`tree`]); [`ExtentManager`] wraps it in the position-③ lock
//! (report §5.1), delegates every operation, and doubles as the `PageCache`
//! backend, mirroring ext2's `InodeBlockManager` over `BlockPtrTree`.
//!
//! Interior tree nodes are read per lookup (through the metadata-read
//! funnel); a frame cache (an allocation-free fast path for repeated lookups)
//! is a P9 optimization.

use core::sync::atomic::{AtomicUsize, Ordering};

use super::{super::prelude::*, RAW_BLOCK_PTRS_LEN};

mod node;
mod tree;

pub(super) use self::tree::ExtentTree;

/// State of a mapped logical block — the three-way view tests assert against
/// (production code pattern-matches [`Mapping`] directly).
#[cfg(ktest)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MapState {
    /// Backed by written data on disk.
    Written,
    /// Allocated but never written; reads as zeros.
    Unwritten,
    /// Not allocated; reads as zeros.
    Hole,
}

/// The result of mapping a logical block. A hole carries no physical block at
/// all — there is no in-band "pblock 0" to misread as block 0.
#[derive(Clone, Copy, Debug)]
pub(super) enum Mapping {
    /// A contiguous mapped physical run.
    Mapped {
        pblock: Ext4Bid,
        /// Contiguous logical blocks from the queried one to the run's end.
        #[cfg_attr(not(ktest), expect(dead_code))]
        len: u32,
        /// `false` = preallocated-unwritten: allocated, but reads as zeros.
        written: bool,
    },
    /// Not allocated; reads as zeros.
    Hole {
        /// Logical blocks known to be unmapped (currently always 1). Only the
        /// test view reads it today; the read path zero-fills a hole one block
        /// at a time.
        #[cfg_attr(not(ktest), expect(dead_code))]
        len: u32,
    },
}

impl Mapping {
    /// Returns the three-way state view (see [`MapState`]).
    #[cfg(ktest)]
    pub(super) const fn state(&self) -> MapState {
        match self {
            Mapping::Mapped { written: true, .. } => MapState::Written,
            Mapping::Mapped { written: false, .. } => MapState::Unwritten,
            Mapping::Hole { .. } => MapState::Hole,
        }
    }

    /// Returns the number of contiguous logical blocks this mapping describes.
    #[cfg(ktest)]
    pub(super) const fn len(&self) -> u32 {
        match self {
            Mapping::Mapped { len, .. } | Mapping::Hole { len } => *len,
        }
    }

    /// Returns the physical block backing the run, `None` for a hole.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(super) const fn mapped_pblock(&self) -> Option<Ext4Bid> {
        match self {
            Mapping::Mapped { pblock, .. } => Some(*pblock),
            Mapping::Hole { .. } => None,
        }
    }

    /// Returns whether reading these blocks must return zeros without device I/O.
    #[cfg(ktest)]
    pub(super) const fn reads_as_zeros(&self) -> bool {
        !matches!(self, Mapping::Mapped { written: true, .. })
    }
}

/// Maps an inode's logical blocks to physical blocks via its [`ExtentTree`],
/// which owns the authoritative tree + `i_blocks` accounting.
///
/// Thin delegation over the tree: this type contributes the lock (position ③
/// in the global order, report §5.1), the filesystem back-reference, and the
/// `PageCache` backend surface — mirroring ext2's `InodeBlockManager` over
/// `BlockPtrTree`.
pub(super) struct ExtentManager {
    /// The authoritative extent tree (the ③ "ExtentTree" lock).
    state: RwMutex<ExtentTree>,
    /// Cached page count for the `PageCache` backend.
    npages: AtomicUsize,
    /// Back-reference to the filesystem, for the block device (metadata and
    /// data reads).
    fs: Weak<super::super::fs::Ext4>,
}

impl ExtentManager {
    /// Validates `root` (see [`ExtentTree::try_new`]) and builds the manager.
    pub(super) fn try_new(
        root: [u32; RAW_BLOCK_PTRS_LEN],
        sector_count: u64,
        fs: Weak<super::super::fs::Ext4>,
        npages: usize,
    ) -> Result<Self> {
        Ok(Self {
            state: RwMutex::new(ExtentTree::try_new(root, sector_count)?),
            npages: AtomicUsize::new(npages),
            fs,
        })
    }

    /// Returns a strong reference to the owning filesystem.
    fn fs(&self) -> Result<Arc<super::super::fs::Ext4>> {
        self.fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "filesystem dropped"))
    }

    /// Maps logical block `iblock` to a physical run.
    ///
    /// The returned length spans from `iblock` to the end of the covering
    /// extent, so callers can batch contiguous reads.
    pub(super) fn map_blocks(&self, iblock: Iblock) -> Result<Mapping> {
        let fs = self.fs()?;
        let tree = self.state.read();

        match tree.lookup(&fs, iblock)? {
            Some(extent) => {
                let offset_in_extent = iblock - extent.block();
                Ok(Mapping::Mapped {
                    pblock: extent.start() + offset_in_extent as Ext4Bid,
                    len: extent.len() as u32 - offset_in_extent,
                    written: !extent.is_unwritten(),
                })
            }
            None => Ok(Mapping::Hole { len: 1 }),
        }
    }

    /// Returns the inode's `i_blocks` (512-byte sectors) accounting.
    pub(super) fn sector_count(&self) -> u64 {
        self.state.read().sector_count()
    }
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
            return_errno_with_message!(Errno::EINVAL, "read past end of inode");
        }
        let iblock = Iblock::try_from(idx)
            .map_err(|_| Error::with_message(Errno::EINVAL, "logical block number overflow"))?;

        let Mapping::Mapped {
            pblock,
            written: true,
            ..
        } = self.map_blocks(iblock)?
        else {
            // Holes and unwritten extents read as zeros without device I/O.
            complete_fn(BioStatus::Zeros);
            return Ok(());
        };

        let fs = self
            .fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "filesystem dropped"))?;
        fs.read_blocks_async(pblock, bio_segment, Some(complete_fn), io_batch)
    }

    fn submit_write_bio(
        &self,
        _idx: usize,
        _bio_segment: BioSegment,
        _complete_fn: BioCompleteFn,
        _io_batch: &mut IoBatch,
    ) -> Result<()> {
        // Read-only mount: the `PageCache` backend trait requires this method,
        // but no dirty page can reach it (all write paths return `EROFS`).
        return_errno_with_message!(Errno::EROFS, "read-only ext4: block writeback disabled");
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    use super::{
        super::super::test_utils::Ext4FixtureBuilder,
        node::{EXTENT_MAGIC, RawExtent, RawExtentHeader},
        *,
    };

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
        bytes[0..12].copy_from_slice(header.as_bytes());
        for (i, extent) in extents.iter().enumerate() {
            let off = 12 * (1 + i);
            bytes[off..off + 12].copy_from_slice(extent.as_bytes());
        }
        block
    }

    #[ktest]
    fn map_written_extent() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        let root = inline_root(&[RawExtent {
            block: 0,
            len: 4,
            start_hi: 0,
            start_lo: 100,
        }]);
        let em = ExtentManager::try_new(root, 4 * 8, f.ext4.this(), 4).unwrap();

        let m0 = em.map_blocks(0).unwrap();
        assert_eq!(m0.state(), MapState::Written);
        assert_eq!(m0.mapped_pblock(), Some(100));
        assert_eq!(m0.len(), 4);

        // Mapping from the middle returns the remaining run.
        let m2 = em.map_blocks(2).unwrap();
        assert_eq!(m2.mapped_pblock(), Some(102));
        assert_eq!(m2.len(), 2);

        // Past the extent: a hole.
        let m4 = em.map_blocks(4).unwrap();
        assert_eq!(m4.state(), MapState::Hole);
        assert!(m4.reads_as_zeros());
    }

    #[ktest]
    fn map_unwritten_extent_reads_as_zeros() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        let root = inline_root(&[RawExtent {
            block: 0,
            len: 32768 + 2, // unwritten, length 2
            start_hi: 0,
            start_lo: 500,
        }]);
        let em = ExtentManager::try_new(root, 2 * 8, f.ext4.this(), 2).unwrap();
        let m = em.map_blocks(0).unwrap();
        assert_eq!(m.state(), MapState::Unwritten);
        assert!(m.reads_as_zeros());
        assert_eq!(m.mapped_pblock(), Some(500));
    }

    // A1-B0 read regression: the two `journaled_*` tests that formerly lived
    // here (`journaled_tree_read_sees_uncheckpointed_leaf`,
    // `journaled_reserialize_captures_reused_leaf`) staged a
    // committed-but-un-checkpointed leaf via `ensure_allocated` + a journal
    // commit, then asserted the read was served from the journal's after-image.
    // That stale-read window cannot exist on a read-only, journal-less mount —
    // the device is always authoritative — so the tests are cut. The multi-node
    // *device-parse* read they also exercised is preserved by
    // `tree::tests::descends_into_external_leaf`, which builds a depth-1 index
    // root over an external leaf laid down on the device and descends into it
    // through `lookup`.
}
