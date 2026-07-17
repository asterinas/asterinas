// SPDX-License-Identifier: MPL-2.0

//! Ext4 block-group descriptors and allocation domains.
//!
//! Each block group owns its block and inode allocation bitmaps. Dirty group
//! metadata is written back with read-modify-write so unmodeled descriptor
//! fields are preserved. A per-group inode cache gives every live inode a
//! stable identity and lets filesystem sync flush loaded inodes.
//!
//! # Locking
//!
//! `BlockGroup` uses two independent locks:
//!
//! - `metadata` — protects the group descriptor, the block bitmap, and the
//!   inode bitmap. Held briefly during alloc/free operations; the inode bitmap
//!   lives under this same lock, so the inode allocator introduces no new lock
//!   acquisition order.
//! - `inode_cache` — protects the per-group live inode map. Uses double-checked
//!   locking (read then promote to write on miss). Never held while syncing an
//!   inode (see [`BlockGroup::sync_inodes`]).

use core::fmt;

use super::{
    fs::Ext4,
    inode::{Inode, InodeDesc, RawInode},
    prelude::*,
    super_block::SuperBlock,
    utils,
};

/// A block group's allocation domain.
///
/// Owns the cached block bitmap, inode bitmap, and group descriptor behind a
/// single lock, plus the geometry needed to allocate/free blocks and inodes and
/// write metadata back to disk.
pub(super) struct BlockGroup {
    /// Block group index (0-based).
    group_idx: usize,
    /// Group descriptor, block bitmap, and inode bitmap, protected by a single
    /// lock.
    metadata: RwMutex<BlockGroupMetadata>,
    /// Backing block device (shared with `Ext4` and other groups).
    block_device: Arc<dyn BlockDevice>,
    /// Cached geometry: first filesystem-wide block number of this group.
    first_block: Ext4Bid,
    /// Cached geometry: last filesystem-wide block number of this group.
    last_block: Ext4Bid,
    /// Cached geometry: inode table blocks per group.
    nr_inode_table_blocks_per_group: u32,
    /// Cached geometry: inodes per group.
    nr_inodes_per_group: u32,
    /// Cached geometry: inode size in bytes.
    inode_size: usize,
    /// Cached geometry: filesystem block size in bytes.
    block_size: usize,
    /// Absolute byte offset of this group's `RawBlockGroup` in the GDT.
    desc_offset: usize,
    /// Per-group live inode cache keyed by group-local inode index.
    ///
    /// Ext4 keeps this cache locally because the VFS layer does not provide a
    /// shared inode cache for filesystem implementations. It gives inodes a
    /// stable identity and lets the filesystem enumerate every dirty inode for a
    /// consistent flush at sync/unmount time.
    inode_cache: RwMutex<BTreeMap<u16, Arc<Inode>>>,
    /// Serializes inode-slot writes into this group's inode table.
    ///
    /// Slot writes are sub-sector, so the block layer turns each into a
    /// sector read-modify-write; two concurrent writes to slots sharing a
    /// sector would otherwise lose one update. A leaf lock: only device I/O
    /// happens while it is held.
    inode_table_lock: Mutex<()>,
}

impl Debug for BlockGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlockGroup")
            .field("group_idx", &self.group_idx)
            .finish()
    }
}

impl BlockGroup {
    /// Loads a block group from the descriptor table.
    ///
    /// Reads and decodes the group's `RawBlockGroup` at
    /// `gdt_base_offset + group_idx * size_of::<RawBlockGroup>()`, caches the
    /// group's geometry from `sb`, and loads the block bitmap.
    ///
    /// Loading is lenient: strict validation that the system-metadata blocks are
    /// marked allocated in the bitmap is deferred (the read-only fixtures carry
    /// an all-zero bitmap and must still mount).
    pub(super) fn load(
        device: Arc<dyn BlockDevice>,
        group_idx: usize,
        sb: &SuperBlock,
        gdt_base_offset: usize,
    ) -> Result<Self> {
        let desc_offset = group_idx
            .checked_mul(size_of::<RawBlockGroup>())
            .and_then(|offset| gdt_base_offset.checked_add(offset))
            .ok_or_else(|| {
                Error::with_message(Errno::EOVERFLOW, "group descriptor offset overflow")
            })?;
        let raw_group = device
            .read_val::<RawBlockGroup>(desc_offset)
            .map_err(|_| Error::with_message(Errno::EIO, "failed to read group descriptor"))?;
        let desc = BlockGroupDesc::from(&raw_group);

        // Cache geometry from `SuperBlock` at load time.
        let nr_blocks_per_group = Ext4Bid::from(sb.nr_blocks_per_group());
        let group_idx_bid = Ext4Bid::try_from(group_idx)
            .map_err(|_| Error::with_message(Errno::EOVERFLOW, "block group index overflow"))?;
        let first_block = group_idx_bid
            .checked_mul(nr_blocks_per_group)
            .and_then(|offset| sb.first_data_block().checked_add(offset))
            .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "block group offset overflow"))?;
        let nr_block_groups = usize::try_from(sb.nr_block_groups())
            .map_err(|_| Error::with_message(Errno::EOVERFLOW, "block group count overflow"))?;
        let last_block = if group_idx == nr_block_groups - 1 {
            sb.total_blocks() - 1
        } else {
            first_block + nr_blocks_per_group - 1
        };
        let nr_inode_table_blocks_per_group = sb.nr_inode_table_blocks_per_group();
        let nr_inodes_per_group = sb.nr_inodes_per_group();
        let inode_size = sb.inode_size();
        let block_size = sb.block_size();

        // Load the block bitmap and the inode bitmap.
        let block_bitmap =
            Self::load_block_bitmap(device.as_ref(), first_block, last_block, &desc)?;
        let inode_bitmap = Self::load_inode_bitmap(device.as_ref(), nr_inodes_per_group, &desc)?;

        Ok(Self {
            group_idx,
            metadata: RwMutex::new(BlockGroupMetadata {
                desc: Dirty::new(desc),
                block_bitmap: Dirty::new(block_bitmap),
                inode_bitmap: Dirty::new(inode_bitmap),
            }),
            block_device: device,
            first_block,
            last_block,
            nr_inode_table_blocks_per_group,
            nr_inodes_per_group,
            inode_size,
            block_size,
            desc_offset,
            inode_cache: RwMutex::new(BTreeMap::new()),
            inode_table_lock: Mutex::new(()),
        })
    }

    /// Returns whether `ino` is marked allocated in this group's inode bitmap.
    ///
    /// The reclaim path checks this to avoid freeing an inode twice.
    pub(super) fn is_inode_allocated(&self, ino: Ext4Ino) -> bool {
        let inode_idx = self.inode_idx_in_group(ino);
        self.metadata.read().inode_bitmap.is_allocated(inode_idx)
    }

    /// Looks up an inode by inode number through this group's inode cache.
    ///
    /// Returns the same `Arc<Inode>` for repeated lookups of one inode number,
    /// so concurrent users share one in-memory inode (and one set of dirty
    /// state). The fast path hits the cache under the read lock; the slow path
    /// promotes to the write lock, re-checks (another thread may have inserted
    /// the inode in the gap), then loads the descriptor from disk and inserts it.
    pub(super) fn lookup_inode(&self, ino: Ext4Ino, fs: Weak<Ext4>) -> Result<Arc<Inode>> {
        let inode_idx = self.inode_idx_in_group(ino);

        // Fast path: cache hit under the read lock.
        if let Some(inode) = self.inode_cache.read().get(&inode_idx) {
            return Ok(inode.clone());
        }

        // Slow path: revalidate under the write lock, since another thread may
        // have inserted the inode between the read and write lock acquisition.
        let mut inode_cache = self.inode_cache.write();
        if let Some(inode) = inode_cache.get(&inode_idx) {
            return Ok(inode.clone());
        }

        let desc = self.read_inode_desc(ino)?;
        let type_ = desc.type_();
        let inode = Inode::new(ino, type_, Dirty::new(desc), self.group_idx, fs)?;
        inode_cache.insert(inode_idx, inode.clone());
        Ok(inode)
    }

    /// Inserts a newly created inode into this group's live cache.
    pub(super) fn insert_inode(&self, inode: Arc<Inode>) {
        let inode_idx = self.inode_idx_in_group(inode.ino());
        self.inode_cache.write().insert(inode_idx, inode);
    }

    /// Removes one inode from this group's live cache.
    pub(super) fn remove_inode(&self, ino: Ext4Ino) -> Option<Arc<Inode>> {
        let inode_idx = self.inode_idx_in_group(ino);
        self.inode_cache.write().remove(&inode_idx)
    }

    /// Flushes every cached inode's xattr block, data pages, and metadata back
    /// to disk.
    ///
    /// The `Arc<Inode>` handles are cloned out under the read lock, which is then
    /// dropped *before* any inode is synced. This drop-before-sync ordering is
    /// required: `Inode::sync_data_and_meta` acquires `inner.write()`, so holding
    /// `inode_cache.read()` across the sync would invert the lock order against
    /// create/unlink paths that take `inner.write()` first and `inode_cache`
    /// after.
    pub(super) fn sync_inodes(&self) -> Result<()> {
        let inodes: Vec<Arc<Inode>> = self.inode_cache.read().values().cloned().collect();
        for inode in inodes {
            inode.sync_all_no_barrier()?;
        }
        Ok(())
    }

    /// Locks this group's inode table for a slot read-modify-write.
    pub(super) fn lock_inode_table(&self) -> MutexGuard<'_, ()> {
        self.inode_table_lock.lock()
    }

    /// Returns a read guard over the combined group metadata.
    #[cfg(ktest)]
    pub(super) fn metadata(&self) -> RwMutexReadGuard<'_, BlockGroupMetadata> {
        self.metadata.read()
    }

    /// Returns the first filesystem-wide block number of this group.
    pub(super) fn first_block(&self) -> Ext4Bid {
        self.first_block
    }

    /// Returns the last filesystem-wide block number of this group.
    pub(super) fn last_block(&self) -> Ext4Bid {
        self.last_block
    }

    /// Returns the number of free blocks in this group.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(super) fn free_blocks_count(&self) -> u32 {
        self.metadata.read().desc.free_blocks_count()
    }

    /// Returns the number of free inodes in this group.
    #[cfg(ktest)]
    pub(super) fn free_inodes_count(&self) -> u32 {
        self.metadata.read().desc.free_inodes_count()
    }

    /// Returns the number of in-use directory inodes in this group.
    #[cfg(ktest)]
    pub(super) fn used_dirs_count(&self) -> u32 {
        self.metadata.read().desc.used_dirs_count()
    }

    /// Returns the starting block of this group's inode table.
    pub(super) fn inode_table_bid(&self) -> Ext4Bid {
        self.metadata.read().desc.inode_table_bid()
    }

    /// Attempts to allocate up to `count` contiguous blocks within this group.
    ///
    /// Returns `Ok(range)` with filesystem-wide block numbers on success, or an
    /// empty range (`Ok(0..0)`) if the group has no allocatable blocks. Returns
    /// `Err(EIO)` on bitmap/counter corruption.
    pub(super) fn alloc_blocks(&self, count: u32, sb_free_blocks: u64) -> Result<Range<Ext4Bid>> {
        let group_size = u32::try_from(self.last_block - self.first_block + 1)
            .map_err(|_| Error::with_message(Errno::EOVERFLOW, "block group is too large"))?;
        debug_assert!(group_size <= u32::from(IdBitmap::capacity()));

        let mut metadata = self.metadata.write();

        let requested_count = count
            .min(group_size)
            .min(metadata.desc.free_blocks_count())
            .min(u32::try_from(sb_free_blocks.min(u64::from(u32::MAX))).expect("value is clamped"));
        let mut requested_count = u16::try_from(requested_count)
            .map_err(|_| Error::with_message(Errno::EOVERFLOW, "block request is too large"))?;

        // TODO: Improve bitmap allocation to reduce fragmentation (e.g., find the
        // first free block directly instead of retrying with smaller counts).
        let mut allocated_range = None;
        while requested_count > 0 {
            let candidate_range = metadata.block_bitmap.alloc_consecutive(requested_count);
            if candidate_range.is_some() {
                allocated_range = candidate_range;
                break;
            }
            requested_count /= 2;
        }

        let Some(range) = allocated_range else {
            if metadata.desc.free_blocks_count() > 0 {
                return_errno_with_message!(Errno::EIO, "block bitmap corruption detected");
            }
            return Ok(0..0);
        };

        let range_start = Ext4Bid::from(range.start);
        let alloc_count = u32::try_from(range.len()).expect("bitmap range length fits u32");

        let abs_range = (self.first_block + range_start)
            ..(self.first_block + range_start + Ext4Bid::from(alloc_count));
        if self.overlaps_system_zone_with(&metadata.desc, abs_range)
            || metadata.desc.free_blocks_count() < alloc_count
            || sb_free_blocks < u64::from(alloc_count)
        {
            metadata.block_bitmap.free_consecutive(range);
            return_errno_with_message!(Errno::EIO, "block bitmap corruption detected");
        }

        let new_free = metadata.desc.free_blocks_count() - alloc_count;
        metadata.desc.free_blocks_count = new_free;

        let range_start_block = self.first_block + Ext4Bid::from(range.start);
        let range_end_block = self.first_block + Ext4Bid::from(range.end);
        Ok(range_start_block..range_end_block)
    }

    /// Frees a contiguous range of group-relative block bits.
    ///
    /// Returns the number of blocks actually freed (allocated-to-free
    /// transitions). Returns `Err(EIO)` when the range overlaps the group's
    /// system zone.
    pub(super) fn free_blocks(&self, bit_range: Range<u32>) -> Result<u32> {
        let start_bit = bit_range.start;
        let group_count = bit_range
            .end
            .checked_sub(bit_range.start)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid block range"))?;
        // Validate system zone overlap using filesystem-wide coordinates.
        let abs_range = (self.first_block + Ext4Bid::from(start_bit))
            ..(self.first_block + Ext4Bid::from(bit_range.end));

        let mut metadata = self.metadata.write();

        if self.overlaps_system_zone_with(&metadata.desc, abs_range) {
            return_errno_with_message!(Errno::EIO, "freeing blocks in system zone");
        }

        // Clear bits one by one and count only allocated-to-free transitions.
        let range_start = u16::try_from(start_bit)
            .map_err(|_| Error::with_message(Errno::EINVAL, "block index exceeds bitmap"))?;
        let range_end = u16::try_from(start_bit + group_count)
            .map_err(|_| Error::with_message(Errno::EINVAL, "block range exceeds bitmap"))?;
        let mut actually_freed: u32 = 0;
        for block_bit in range_start..range_end {
            if !metadata.block_bitmap.is_allocated(block_bit) {
                warn!(
                    "free_blocks: bit already cleared for block {}",
                    self.first_block
                        + Ext4Bid::from(start_bit)
                        + Ext4Bid::from(block_bit - range_start)
                );
            } else {
                metadata.block_bitmap.free(block_bit);
                actually_freed += 1;
            }
        }

        let new_free = metadata
            .desc
            .free_blocks_count()
            .checked_add(actually_freed)
            .ok_or_else(|| Error::with_message(Errno::EIO, "free block count overflow in group"))?;
        metadata.desc.free_blocks_count = new_free;

        Ok(actually_freed)
    }

    /// Attempts to allocate one inode within this group.
    ///
    /// Allocates a single free bit in the inode bitmap, decrements the group's
    /// free-inode counter, and (for directories) increments `used_dirs_count`.
    /// Returns `Some(inode_idx)` with the 0-based group-local inode index, or
    /// `None` if this group has no free inode. Mirrors [`Self::alloc_blocks`] on the
    /// block side.
    pub(super) fn alloc_ino(&self, type_: InodeType) -> Result<Option<u32>> {
        let mut metadata = self.metadata.write();

        if metadata.desc.free_inodes_count() == 0 {
            return Ok(None);
        }
        if type_.is_directory() && metadata.desc.used_dirs_count() == u32::from(u16::MAX) {
            return_errno_with_message!(Errno::EIO, "group used directory counter overflow");
        }

        // Allocate exactly one free inode bit.
        let Some(range) = metadata.inode_bitmap.alloc_consecutive(1) else {
            // The counter said there was a free inode but the bitmap had none.
            return_errno_with_message!(Errno::EIO, "inode bitmap corruption detected");
        };
        let inode_idx = u32::from(range.start);

        metadata.desc.free_inodes_count = metadata.desc.free_inodes_count() - 1;
        if type_.is_directory() {
            metadata.desc.used_dirs_count = metadata.desc.used_dirs_count() + 1;
        }

        Ok(Some(inode_idx))
    }

    /// Frees one inode within this group, by its group-local index.
    ///
    /// Clears the inode bitmap bit, increments the group's free-inode counter,
    /// and (for directories) decrements `used_dirs_count`. Returns `true` if the
    /// bit transitioned allocated-to-free, `false` if it was already clear (logs
    /// a warning, mirroring [`Self::free_blocks`]).
    pub(super) fn free_inode(&self, group_local_idx: u32, type_: InodeType) -> Result<bool> {
        let mut metadata = self.metadata.write();

        let inode_bit = u16::try_from(group_local_idx)
            .map_err(|_| Error::with_message(Errno::EINVAL, "inode index exceeds bitmap"))?;
        if !metadata.inode_bitmap.is_allocated(inode_bit) {
            warn!(
                "free_inode: inode bit {} already cleared in group {}",
                group_local_idx, self.group_idx
            );
            return Ok(false);
        }

        let new_free = metadata
            .desc
            .free_inodes_count()
            .checked_add(1)
            .ok_or_else(|| Error::with_message(Errno::EIO, "free inode count overflow in group"))?;
        let new_used_dirs = if type_.is_directory() {
            Some(
                metadata
                    .desc
                    .used_dirs_count()
                    .checked_sub(1)
                    .ok_or_else(|| {
                        Error::with_message(Errno::EIO, "used directory counter underflow in group")
                    })?,
            )
        } else {
            None
        };

        metadata.inode_bitmap.free(inode_bit);
        metadata.desc.free_inodes_count = new_free;
        if let Some(new_used_dirs) = new_used_dirs {
            metadata.desc.used_dirs_count = new_used_dirs;
        }

        Ok(true)
    }

    /// Loads and decodes an inode's on-disk descriptor from the inode table.
    ///
    /// Inode-table reads go directly to the block device. The inode cache is
    /// used for identity and enumeration, not for inode-table pages.
    pub(super) fn read_inode_desc(&self, ino: Ext4Ino) -> Result<InodeDesc> {
        let idx_in_group = usize::from(self.inode_idx_in_group(ino));
        let inode_table_offset = utils::block_offset(self.inode_table_bid(), self.block_size)?;
        let inode_slot_offset = idx_in_group
            .checked_mul(self.inode_size)
            .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "inode slot offset overflow"))?;
        let offset = inode_table_offset
            .checked_add(inode_slot_offset)
            .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "inode table offset overflow"))?;
        let raw = RawInode::read_from_slot(&self.block_device, offset, self.inode_size)?;
        InodeDesc::try_from(&raw)
    }

    /// Writes dirty metadata back to disk under a single lock.
    ///
    /// Both bitmaps are written in full. The group descriptor is updated via
    /// read-modify-write: the raw descriptor is read, only the mutated counters
    /// (`free_blocks_count_lo`, `free_inodes_count_lo`, `used_dirs_count_lo`) are
    /// patched, and the result is written back so every other on-disk field
    /// (flags, csum, exclude, itable_unused) is preserved.
    pub(super) fn sync_metadata(&self) -> Result<()> {
        let mut metadata = self.metadata.write();

        if metadata.block_bitmap.is_dirty() {
            let block_bitmap_bid = metadata.desc.block_bitmap_bid();
            if self
                .block_device
                .write_bytes(
                    Bid::new(block_bitmap_bid).to_offset(),
                    metadata.block_bitmap.as_bytes(),
                )
                .is_err()
            {
                // Keep the dirty bit set on writeback failure for retry.
                return_errno_with_message!(Errno::EIO, "failed to write block bitmap");
            }
            metadata.block_bitmap.clear_dirty();
        }

        if metadata.inode_bitmap.is_dirty() {
            let inode_bitmap_bid = metadata.desc.inode_bitmap_bid();
            if self
                .block_device
                .write_bytes(
                    Bid::new(inode_bitmap_bid).to_offset(),
                    metadata.inode_bitmap.as_bytes(),
                )
                .is_err()
            {
                // Keep the dirty bit set on writeback failure for retry.
                return_errno_with_message!(Errno::EIO, "failed to write inode bitmap");
            }
            metadata.inode_bitmap.clear_dirty();
        }

        if metadata.desc.is_dirty() {
            let mut raw = self
                .block_device
                .read_val::<RawBlockGroup>(self.desc_offset)
                .map_err(|_| {
                    Error::with_message(Errno::EIO, "failed to read group descriptor for sync")
                })?;
            raw.free_blocks_count_lo =
                u16::try_from(metadata.desc.free_blocks_count()).map_err(|_| {
                    Error::with_message(Errno::EOVERFLOW, "free block count exceeds disk field")
                })?;
            raw.free_inodes_count_lo =
                u16::try_from(metadata.desc.free_inodes_count()).map_err(|_| {
                    Error::with_message(Errno::EOVERFLOW, "free inode count exceeds disk field")
                })?;
            raw.used_dirs_count_lo =
                u16::try_from(metadata.desc.used_dirs_count()).map_err(|_| {
                    Error::with_message(Errno::EOVERFLOW, "directory count exceeds disk field")
                })?;
            self.block_device
                .write_val(self.desc_offset, &raw)
                .map_err(|_| Error::with_message(Errno::EIO, "failed to write group descriptor"))?;
            metadata.desc.clear_dirty();
        }

        Ok(())
    }

    /// Returns the 0-based group-local inode index for `ino`.
    fn inode_idx_in_group(&self, ino: Ext4Ino) -> u16 {
        debug_assert!(ino > 0);
        debug_assert_eq!(
            usize::try_from((ino - 1) / self.nr_inodes_per_group)
                .expect("inode group index fits usize"),
            self.group_idx
        );
        u16::try_from((ino - 1) % self.nr_inodes_per_group)
            .expect("group-local inode index fits u16")
    }

    /// Loads the block bitmap for this group.
    fn load_block_bitmap(
        block_device: &dyn BlockDevice,
        first_block: Ext4Bid,
        last_block: Ext4Bid,
        desc: &BlockGroupDesc,
    ) -> Result<IdBitmap> {
        let bitmap_bid = desc.block_bitmap_bid();

        let mut buf = vec![0u8; BLOCK_SIZE];
        if block_device
            .read_bytes(Bid::new(bitmap_bid).to_offset(), &mut buf)
            .is_err()
        {
            return_errno_with_message!(Errno::EIO, "failed to read block bitmap");
        }

        let capacity = u16::try_from(last_block - first_block + 1).map_err(|_| {
            Error::with_message(Errno::EINVAL, "block group exceeds bitmap capacity")
        })?;
        debug_assert!(capacity <= IdBitmap::capacity());
        Ok(IdBitmap::from_buf(buf.into_boxed_slice(), capacity))
    }

    /// Loads the inode bitmap for this group.
    ///
    /// The bitmap's logical capacity is the number of inodes per group, capped
    /// at the bitmap's physical capacity (a single block always holds at least
    /// as many bits as inodes a group can have).
    fn load_inode_bitmap(
        block_device: &dyn BlockDevice,
        nr_inodes_per_group: u32,
        desc: &BlockGroupDesc,
    ) -> Result<IdBitmap> {
        let bitmap_bid = desc.inode_bitmap_bid();

        let mut buf = vec![0u8; BLOCK_SIZE];
        if block_device
            .read_bytes(Bid::new(bitmap_bid).to_offset(), &mut buf)
            .is_err()
        {
            return_errno_with_message!(Errno::EIO, "failed to read inode bitmap");
        }

        let capacity = u16::try_from(nr_inodes_per_group.min(u32::from(IdBitmap::capacity())))
            .expect("inode bitmap capacity fits u16");
        Ok(IdBitmap::from_buf(buf.into_boxed_slice(), capacity))
    }

    /// Checks whether `range` (filesystem-wide block numbers) overlaps any
    /// system-metadata block of this group: the block bitmap, the inode bitmap,
    /// or the inode-table blocks.
    fn overlaps_system_zone_with(&self, desc: &BlockGroupDesc, range: Range<Ext4Bid>) -> bool {
        if range.is_empty() {
            return false;
        }

        let block_bitmap = desc.block_bitmap_bid()..(desc.block_bitmap_bid() + 1);
        let inode_bitmap = desc.inode_bitmap_bid()..(desc.inode_bitmap_bid() + 1);
        let inode_table = desc.inode_table_bid()
            ..(desc.inode_table_bid() + Ext4Bid::from(self.nr_inode_table_blocks_per_group));

        Self::ranges_overlap(&range, &block_bitmap)
            || Self::ranges_overlap(&range, &inode_bitmap)
            || Self::ranges_overlap(&range, &inode_table)
    }

    fn ranges_overlap(a: &Range<Ext4Bid>, b: &Range<Ext4Bid>) -> bool {
        !a.is_empty() && !b.is_empty() && a.start < b.end && b.start < a.end
    }
}

/// One block group's metadata: the descriptor, the block bitmap, and the inode
/// bitmap.
///
/// All three members carry dirty tracking; writeback is deferred to
/// [`BlockGroup::sync_metadata`].
pub(super) struct BlockGroupMetadata {
    /// Group descriptor with dirty tracking.
    pub desc: Dirty<BlockGroupDesc>,
    /// Block bitmap cached in memory.
    pub block_bitmap: Dirty<IdBitmap>,
    /// Inode bitmap cached in memory.
    pub inode_bitmap: Dirty<IdBitmap>,
}

impl Debug for BlockGroupMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlockGroupMetadata")
            .field("desc", &self.desc)
            .field("block_bitmap_dirty", &self.block_bitmap.is_dirty())
            .field("inode_bitmap_dirty", &self.inode_bitmap.is_dirty())
            .finish()
    }
}

/// Validated, Rust-typed block-group descriptor.
///
/// Block numbers use `Ext4Bid`, while the unsupported `64BIT` high halves are
/// not decoded.
#[derive(Clone, Copy, Debug)]
pub(super) struct BlockGroupDesc {
    block_bitmap_bid: Ext4Bid,
    inode_bitmap_bid: Ext4Bid,
    inode_table_bid: Ext4Bid,
    free_blocks_count: u32,
    free_inodes_count: u32,
    used_dirs_count: u32,
}

impl BlockGroupDesc {
    /// Returns the starting block of this group's inode table.
    pub(super) const fn inode_table_bid(&self) -> Ext4Bid {
        self.inode_table_bid
    }

    pub(super) const fn block_bitmap_bid(&self) -> Ext4Bid {
        self.block_bitmap_bid
    }

    pub(super) const fn inode_bitmap_bid(&self) -> Ext4Bid {
        self.inode_bitmap_bid
    }

    pub(super) const fn free_blocks_count(&self) -> u32 {
        self.free_blocks_count
    }

    pub(super) const fn free_inodes_count(&self) -> u32 {
        self.free_inodes_count
    }

    pub(super) const fn used_dirs_count(&self) -> u32 {
        self.used_dirs_count
    }
}

impl From<&RawBlockGroup> for BlockGroupDesc {
    fn from(raw: &RawBlockGroup) -> Self {
        Self {
            block_bitmap_bid: raw.block_bitmap_lo as Ext4Bid,
            inode_bitmap_bid: raw.inode_bitmap_lo as Ext4Bid,
            inode_table_bid: raw.inode_table_lo as Ext4Bid,
            free_blocks_count: u32::from(raw.free_blocks_count_lo),
            free_inodes_count: u32::from(raw.free_inodes_count_lo),
            used_dirs_count: u32::from(raw.used_dirs_count_lo),
        }
    }
}

/// On-disk block-group descriptor, 32 bytes (without the `64BIT` high halves).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct RawBlockGroup {
    pub block_bitmap_lo: u32,
    pub inode_bitmap_lo: u32,
    pub inode_table_lo: u32,
    pub free_blocks_count_lo: u16,
    pub free_inodes_count_lo: u16,
    pub used_dirs_count_lo: u16,
    /// `bg_flags` (e.g. `INODE_UNINIT`, `BLOCK_UNINIT`, `INODE_ZEROED`).
    pub flags: u16,
    pub exclude_bitmap_lo: u32,
    pub block_bitmap_csum_lo: u16,
    pub inode_bitmap_csum_lo: u16,
    pub itable_unused_lo: u16,
    pub checksum: u16,
}

const_assert!(size_of::<RawBlockGroup>() == 32);
