// SPDX-License-Identifier: MPL-2.0

//! Ext4 extent block-mapping engine.
//!
//! It maps a file's logical blocks to physical device blocks by walking an
//! on-disk extent tree rooted inline in the inode's `i_block`. `map_blocks`
//! is the single translation entry point; the `PageCache` backend drives file
//! I/O through it.
//!
//! Interior tree nodes are read directly from the device.

use core::sync::atomic::{AtomicUsize, Ordering};

use super::{
    super::{super::prelude::*, RAW_BLOCK_PTRS_LEN},
    MapState, Mapping,
};

mod node;
mod tree;

/// 512-byte sectors per filesystem block; the unit `i_blocks` is counted in.
const SECTORS_PER_BLOCK: u64 = (BLOCK_SIZE / SECTOR_SIZE) as u64;

/// Mutable extent-tree state for one inode.
///
/// `root` is the inode's 60-byte `i_block` holding the inline extent-tree root;
/// `sector_count` mirrors the inode's `i_blocks` (data + extent-tree metadata,
/// in 512-byte sectors); `dirty` records whether either has changed since the
/// last writeback. All fields are protected by the same lock.
pub(super) struct ExtentTreeState {
    tree: tree::ExtentTree,
    sector_count: u64,
    dirty: bool,
}

/// Maps an inode's logical blocks to physical blocks via its extent tree and
/// owns the tree and `i_blocks` accounting.
pub(in super::super) struct ExtentManager {
    /// The mutable extent-tree state.
    state: RwMutex<ExtentTreeState>,
    /// Cached page count for the `PageCache` backend.
    npages: AtomicUsize,
    /// Back-reference to the filesystem, for the block device and allocator.
    fs: Weak<super::super::super::fs::Ext4>,
}

impl ExtentManager {
    pub(super) fn new(
        root: [u32; RAW_BLOCK_PTRS_LEN],
        sector_count: u64,
        fs: Weak<super::super::super::fs::Ext4>,
        npages: usize,
    ) -> Result<Self> {
        Ok(Self {
            state: RwMutex::new(ExtentTreeState {
                tree: tree::ExtentTree::try_from_root(root)?,
                sector_count,
                dirty: false,
            }),
            npages: AtomicUsize::new(npages),
            fs,
        })
    }

    /// Returns a strong reference to the owning filesystem.
    fn fs(&self) -> Result<Arc<super::super::super::fs::Ext4>> {
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
        let device = fs.block_device().as_ref();
        let state = self.state.read();

        match state.tree.find(device, iblock)? {
            Some(extent) => {
                let offset_in_extent = iblock - extent.block();
                let pblock = extent.start() + offset_in_extent as Ext4Bid;
                let len = u32::from(extent.len()) - offset_in_extent;
                let state = if extent.is_unwritten() {
                    MapState::Unwritten
                } else {
                    MapState::Written
                };
                Ok(Mapping::Mapped { pblock, len, state })
            }
            None => Ok(Mapping::Hole { len: 1 }),
        }
    }

    /// Returns the inode's `i_blocks` (512-byte sectors) accounting.
    pub(super) fn sector_count(&self) -> u64 {
        self.state.read().sector_count
    }

    /// Returns a copy of the inode's 60-byte `i_block` (extent-tree root).
    pub(super) fn root_snapshot(&self) -> [u32; RAW_BLOCK_PTRS_LEN] {
        self.state.read().tree.root()
    }

    /// Returns whether the tree or `i_blocks` has changed since last writeback.
    pub(super) fn is_dirty(&self) -> bool {
        self.state.read().dirty
    }

    /// Clears the dirty flag after a successful inode writeback.
    pub(super) fn clear_dirty(&self) {
        self.state.write().dirty = false;
    }

    /// Updates the cached page-cache capacity bound.
    pub(super) fn set_npages(&self, npages: usize) {
        self.npages.store(npages, Ordering::Release);
    }

    /// Allocates data blocks for every hole in `[start_iblock, end_iblock)`,
    /// converts any unwritten (preallocated) extent overlapping the range to
    /// written, and records both in the extent tree.
    ///
    /// Planning is done from a single snapshot of the current tree: existing
    /// written extents are left untouched (overwrites reuse the mapped block),
    /// unwritten extents in range are flipped to written (so the data the caller
    /// is about to write becomes readable), and blocks are allocated only where
    /// the snapshot showed a true hole. `i_blocks` is grown by every data block
    /// allocated plus the net extent-tree metadata blocks; the conversion adds
    /// no data sectors (the blocks were already counted at allocation time).
    ///
    /// On an allocation error mid-way the partial allocation stays in `state`;
    /// the caller's `rollback_write` truncates it away. A successful conversion
    /// that precedes a failed page-cache write also stays (the blocks were
    /// already allocated, so nothing leaks): leaving them written is benign.
    pub(super) fn ensure_allocated(&self, start_iblock: Iblock, end_iblock: Iblock) -> Result<()> {
        if start_iblock >= end_iblock {
            return Ok(());
        }
        let fs = self.fs()?;
        let device = fs.block_device().as_ref();
        let mut s = self.state.write();

        // Plan hole runs from a snapshot of the current tree by interval-
        // subtracting the existing (sorted, non-overlapping) extents.
        let extents = s.tree.extents(device)?;

        // Flip any unwritten extent that overlaps the write range to written so
        // the blocks `submit_write_bio` fills read back the real data. The
        // physical mapping is preserved; only metadata blocks (a split may grow
        // the tree) move, so `i_blocks` changes by the net metadata delta only.
        if extents.iter().any(|e| {
            e.is_unwritten()
                && e.block() < end_iblock
                && e.block() as u64 + e.len() as u64 > start_iblock as u64
        }) {
            let delta = s
                .tree
                .convert_unwritten(&fs, start_iblock, end_iblock - start_iblock)?;
            let net_meta = delta.meta_allocated as i64 - delta.meta_freed as i64;
            s.sector_count =
                (s.sector_count as i64 + net_meta * SECTORS_PER_BLOCK as i64).max(0) as u64;
            s.dirty = true;
        }

        // Re-snapshot after conversion (the tree layout may have changed), then
        // plan holes against the up-to-date extents.
        Self::allocate_holes(
            &fs,
            &mut s,
            start_iblock,
            end_iblock,
            node::ExtentKind::Written,
        )
    }

    /// Allocates blocks for every hole in `[start_iblock, end_iblock)` and
    /// records them as *unwritten* extents, so they read as zeros until
    /// written. Backs `fallocate`; file size is the caller's business.
    ///
    /// Existing extents (written or unwritten) in the range are left as they
    /// are. On a mid-way allocation error the partial allocation stays in
    /// `state`; the caller's `rollback_fallocate` truncates it away.
    pub(super) fn preallocate(&self, start_iblock: Iblock, end_iblock: Iblock) -> Result<()> {
        if start_iblock >= end_iblock {
            return Ok(());
        }
        let fs = self.fs()?;
        let mut s = self.state.write();
        Self::allocate_holes(
            &fs,
            &mut s,
            start_iblock,
            end_iblock,
            node::ExtentKind::Unwritten,
        )
    }

    /// Fills every hole in `[start_iblock, end_iblock)` with freshly allocated
    /// blocks recorded as `kind` extents, updating `i_blocks` accounting.
    fn allocate_holes(
        fs: &Arc<super::super::super::fs::Ext4>,
        s: &mut ExtentTreeState,
        start_iblock: Iblock,
        end_iblock: Iblock,
        kind: node::ExtentKind,
    ) -> Result<()> {
        let device = fs.block_device().as_ref();
        let extents = s.tree.extents(device)?;
        let holes = compute_holes(&extents, start_iblock, end_iblock);

        for hole in holes {
            let mut ib = hole.start;
            // Prefer the physical end of the previous extent for locality.
            let goal = extents
                .iter()
                .rev()
                .find(|e| e.block() < ib)
                .map(|e| e.start() + e.len() as Ext4Bid)
                .unwrap_or(0);
            while ib < hole.end {
                let want = hole.end - ib;
                let range = fs.alloc_blocks(want, goal)?;
                let got = u32::try_from(range.end - range.start).map_err(|_| {
                    Error::with_message(Errno::EOVERFLOW, "allocated extent is too long")
                })?;
                debug_assert!(got > 0 && got <= want);
                // If recording the extent fails, the just-allocated data blocks
                // are not reachable through the inode, so free them here rather
                // than leak them (`rollback_write` only reclaims blocks in the
                // extent tree).
                let len = u16::try_from(got)
                    .map_err(|_| Error::with_message(Errno::EOVERFLOW, "extent is too long"))?;
                let delta = match s.tree.insert(fs, ib, range.start, len, kind) {
                    Ok(delta) => delta,
                    Err(err) => {
                        let _ = fs.free_blocks(range.start, got);
                        return Err(err);
                    }
                };
                let net_meta = delta.meta_allocated as i64 - delta.meta_freed as i64;
                let added_blocks = got as i64 + net_meta;
                s.sector_count =
                    (s.sector_count as i64 + added_blocks * SECTORS_PER_BLOCK as i64) as u64;
                s.dirty = true;
                ib += got;
            }
        }
        Ok(())
    }

    /// Allocates a single data block for logical block `iblock` (assumed a hole)
    /// and records it. Used by the `submit_write_bio` hole fallback.
    fn allocate_one(&self, iblock: Iblock) -> Result<Ext4Bid> {
        let fs = self.fs()?;
        let mut s = self.state.write();
        let range = fs.alloc_blocks(1, 0)?;
        let pblock = range.start;
        let delta = match s
            .tree
            .insert(&fs, iblock, pblock, 1, node::ExtentKind::Written)
        {
            Ok(delta) => delta,
            Err(err) => {
                // Free the just-allocated block rather than leak it.
                let _ = fs.free_blocks(pblock, 1);
                return Err(err);
            }
        };
        let net_meta = delta.meta_allocated as i64 - delta.meta_freed as i64;
        let added_blocks = 1 + net_meta;
        s.sector_count = (s.sector_count as i64 + added_blocks * SECTORS_PER_BLOCK as i64) as u64;
        s.dirty = true;
        Ok(pblock)
    }

    /// Frees every data block and extent-tree metadata block mapping a logical
    /// region at or beyond `new_size` bytes, rewriting the tree and updating
    /// `i_blocks`.
    pub(super) fn truncate_to_byte_len(&self, new_size: usize) -> Result<()> {
        let fs = self.fs()?;
        let device = fs.block_device().as_ref();
        let keep_blocks = new_size.div_ceil(BLOCK_SIZE) as Iblock;
        let mut s = self.state.write();

        let extents = s.tree.extents(device)?;
        // Count old metadata blocks (external leaves) so the net delta is exact.
        let old_meta = s.tree.external_leaf_count(device)?;

        let mut kept: Vec<(Iblock, u16, Ext4Bid, node::ExtentKind)> = Vec::new();
        let mut freed_data: u64 = 0;
        for e in &extents {
            let e_start = e.block();
            let e_end = e_start + e.len() as Iblock;
            if e_end <= keep_blocks {
                kept.push((e.block(), e.len(), e.start(), e.kind()));
                continue;
            }
            if e_start >= keep_blocks {
                // Entire extent is beyond the new size; free all its blocks.
                fs.free_blocks(e.start(), u32::from(e.len()))?;
                freed_data += e.len() as u64;
                continue;
            }
            // The extent straddles `keep_blocks`: keep the head, free the tail.
            let head_len =
                u16::try_from(keep_blocks - e_start).expect("kept extent head length fits u16");
            let tail_len = e.len() - head_len;
            fs.free_blocks(e.start() + Ext4Bid::from(head_len), u32::from(tail_len))?;
            freed_data += tail_len as u64;
            kept.push((e.block(), head_len, e.start(), e.kind()));
        }

        let new_meta = s.tree.rebuild(&fs, &kept)?;
        let net_meta = new_meta as i64 - old_meta as i64;
        let removed_sectors = (freed_data as i64 - net_meta) * SECTORS_PER_BLOCK as i64;
        // `i_blocks` must never drop below zero; `max(0)` saturates, the assert
        // catches a miscounted `sector_count` in debug builds.
        debug_assert!(s.sector_count as i64 >= removed_sectors);
        s.sector_count = (s.sector_count as i64 - removed_sectors).max(0) as u64;
        s.dirty = true;
        Ok(())
    }
}

/// A contiguous run of unmapped logical blocks.
struct HoleRun {
    start: Iblock,
    end: Iblock,
}

/// Computes the hole runs (unmapped logical blocks) within `[start, end)` by
/// interval-subtracting the sorted, non-overlapping `extents`.
fn compute_holes(extents: &[node::Extent], start: Iblock, end: Iblock) -> Vec<HoleRun> {
    let mut holes = Vec::new();
    let mut cursor = start;
    for e in extents {
        let e_start = e.block();
        let e_end = e_start + e.len() as Iblock;
        if e_end <= cursor {
            continue;
        }
        if e_start >= end {
            break;
        }
        if e_start > cursor {
            holes.push(HoleRun {
                start: cursor,
                end: e_start.min(end),
            });
        }
        cursor = cursor.max(e_end);
        if cursor >= end {
            break;
        }
    }
    if cursor < end {
        holes.push(HoleRun { start: cursor, end });
    }
    holes
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

        let mapping = self.map_blocks(iblock)?;
        if mapping.reads_as_zeros() {
            // Holes and unwritten extents read as zeros without device I/O.
            complete_fn(BioStatus::Zeros);
            return Ok(());
        }

        let Some(pblock) = mapping.pblock() else {
            return_errno_with_message!(Errno::EIO, "allocated mapping has no physical block");
        };
        let fs = self
            .fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "filesystem dropped"))?;
        fs.read_blocks_async(pblock, bio_segment, Some(complete_fn), io_batch)
    }

    fn submit_write_bio(
        &self,
        idx: usize,
        bio_segment: BioSegment,
        complete_fn: BioCompleteFn,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        if idx >= self.npages.load(Ordering::Acquire) {
            return_errno_with_message!(Errno::EINVAL, "write past end of inode");
        }
        let iblock = Iblock::try_from(idx)
            .map_err(|_| Error::with_message(Errno::EINVAL, "logical block number overflow"))?;
        let fs = self.fs()?;

        // Buffered writes pre-allocate in `prepare_write`, so the block is
        // usually already mapped (written or unwritten). The hole branch is the
        // defensive fallback for mmap-dirtied pages, which the upper layer does
        // not pre-allocate.
        let mapping = self.map_blocks(iblock)?;
        let pblock = match mapping {
            Mapping::Mapped { pblock, .. } => pblock,
            Mapping::Hole { .. } => self.allocate_one(iblock)?,
        };
        fs.write_blocks_async(pblock, bio_segment, Some(complete_fn), io_batch)
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    use super::{
        super::super::super::test_utils::Ext4FixtureBuilder,
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
        let em = ExtentManager::new(root, 4 * 8, f.ext4.this(), 4).unwrap();

        let m0 = em.map_blocks(0).unwrap();
        assert_eq!(m0.state(), Some(MapState::Written));
        assert_eq!(m0.pblock(), Some(100));
        assert_eq!(m0.len(), 4);

        // Mapping from the middle returns the remaining run.
        let m2 = em.map_blocks(2).unwrap();
        assert_eq!(m2.pblock(), Some(102));
        assert_eq!(m2.len(), 2);

        // Past the extent: a hole.
        let m4 = em.map_blocks(4).unwrap();
        assert_eq!(m4.state(), None);
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
        let em = ExtentManager::new(root, 2 * 8, f.ext4.this(), 2).unwrap();
        let m = em.map_blocks(0).unwrap();
        assert_eq!(m.state(), Some(MapState::Unwritten));
        assert!(m.reads_as_zeros());
        assert_eq!(m.pblock(), Some(500));
    }

    /// Regression: when `allocate_one` allocates a data block but the following
    /// `insert_extent` fails (here the inline→depth-1 grow needs a leaf block and
    /// the disk is out of space), the data block must be freed, not leaked.
    #[ktest]
    fn allocate_one_frees_block_when_insert_fails() {
        // Exactly one free block: enough for the data block, not the tree leaf.
        let f = Ext4FixtureBuilder::new(2048, 256, 2048)
            .with_free_blocks(1)
            .build()
            .unwrap();
        // A full inline root (4 extents); inserting a 5th forces a depth-1 grow.
        let root = inline_root(&[
            RawExtent {
                block: 0,
                len: 1,
                start_hi: 0,
                start_lo: 100,
            },
            RawExtent {
                block: 2,
                len: 1,
                start_hi: 0,
                start_lo: 200,
            },
            RawExtent {
                block: 4,
                len: 1,
                start_hi: 0,
                start_lo: 300,
            },
            RawExtent {
                block: 6,
                len: 1,
                start_hi: 0,
                start_lo: 400,
            },
        ]);
        let em = ExtentManager::new(root, 4 * 8, f.ext4.this(), 8).unwrap();

        let free_before = f.ext4.super_block().free_blocks_count();
        assert_eq!(free_before, 1);

        // The 5th mapping triggers inline→depth-1; the leaf allocation hits ENOSPC.
        assert!(em.allocate_one(8).is_err());

        // The data block allocated before the failed insert was reclaimed.
        assert_eq!(f.ext4.super_block().free_blocks_count(), free_before);
    }
}
