// SPDX-License-Identifier: MPL-2.0

//! Block-group allocation, inode caching, and inode-table I/O.

mod descriptor;
mod metadata;

use core::fmt;

use aster_block::bio::BioCompleteFn;

pub(super) use self::{
    descriptor::{BlockGroupDesc, RawBlockGroup},
    metadata::BlockGroupMetadata,
};
use super::{
    fs::{Ext4, MountFlavor},
    inode::{Inode, InodeDesc, RawInode},
    prelude::*,
    super_block::SuperBlock,
};
use crate::{fs::utils::IdBitmap, vm::page_cache::PageRun};

/// One EXT2/EXT4 allocation domain and its runtime caches.
///
/// A block group is a filesystem-local allocation domain with its own block
/// bitmap, inode bitmap, and inode table. Keeping related blocks and inodes in
/// the same group improves locality on disk.
///
/// The metadata lock protects decoded descriptor counters and both dirty bitmap
/// buffers together. Filesystem allocation takes the superblock state lock first.
/// Geometry is fixed after mount; inode-cache synchronization is independent.
/// Bitmap indices fit in `u16`, and metadata addresses are validated at mount.
pub(super) struct BlockGroup {
    /// Block group index (0-based).
    group_idx: usize,
    /// Group descriptor and bitmaps, protected by a single lock.
    metadata: RwMutex<BlockGroupMetadata>,
    /// Backing block device (shared with `Ext4` and other groups).
    block_device: Arc<dyn BlockDevice>,
    /// Cached geometry: first filesystem-wide block number of this group.
    first_block: u32,
    /// Cached geometry: last filesystem-wide block number of this group.
    last_block: u32,
    /// Cached geometry: inode table blocks per group.
    nr_inode_table_blocks_per_group: u32,
    /// Cached geometry: inodes per group.
    nr_inodes_per_group: u32,
    /// Cached geometry: inode size in bytes.
    inode_size: usize,
    /// Inode table page cache backend.
    _inode_table_backend: Arc<InodeTableBackend>,
    /// Inode table page cache.
    inode_table_cache: PageCache,
    /// Per-group inode cache keyed by group-local inode index.
    ///
    /// Ext4 keeps this cache locally because the VFS layer does not provide
    /// a shared inode cache for filesystem implementations.
    inode_cache: RwMutex<BTreeMap<u16, Arc<Inode>>>,
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
    /// Caches per-group geometry from `SuperBlock` at load time.
    pub(super) fn load(
        group_descs: &USegment,
        group_idx: usize,
        sb: &SuperBlock,
        block_device: Arc<dyn BlockDevice>,
    ) -> Result<Self> {
        let offset = group_idx * size_of::<RawBlockGroup>();
        let raw_group = group_descs
            .read_val::<RawBlockGroup>(offset)
            .map_err(|_| Error::with_message(Errno::EIO, "failed to read group descriptor"))?;
        let group_desc = BlockGroupDesc::from(raw_group);

        // Cache geometry from `SuperBlock` at load time.
        let first_block_no = sb.group_first_block_no(group_idx);
        let last_block_no = sb.group_last_block_no(group_idx);
        let nr_inode_table_blocks_per_group = sb.nr_inode_table_blocks_per_group();
        let nr_inodes_per_group = sb.nr_inodes_per_group();
        let nr_block_groups = sb.nr_block_groups() as usize;
        let nr_inodes_in_group = if group_idx == nr_block_groups - 1 {
            sb.total_inodes() - (group_idx as u32) * nr_inodes_per_group
        } else {
            nr_inodes_per_group
        };
        let inode_size = sb.inode_size();

        group_desc.validate_free_counts(last_block_no - first_block_no + 1, nr_inodes_in_group)?;

        // Load and validate bitmaps once during mount, keep them cached in memory.
        let block_bitmap = Self::load_block_bitmap(
            block_device.as_ref(),
            first_block_no,
            last_block_no,
            &group_desc,
        )?;
        let inode_bitmap =
            Self::load_inode_bitmap(block_device.as_ref(), nr_inodes_per_group, &group_desc)?;

        group_desc.validate_metadata_blocks(
            &block_bitmap,
            first_block_no,
            last_block_no,
            nr_inode_table_blocks_per_group,
        )?;

        // Create `PageCache` for inode table backed by `InodeTableBackend`.
        let raw_inodes_size = (nr_inodes_per_group as usize) * inode_size;
        let backend = Arc::new(InodeTableBackend {
            inode_table_bid: group_desc.inode_table_bid,
            raw_inodes_size,
            block_device: block_device.clone(),
        });
        let inode_table_cache =
            PageCache::new_with_backend(raw_inodes_size, Arc::downgrade(&backend) as _)?;

        Ok(Self {
            group_idx,
            metadata: RwMutex::new(BlockGroupMetadata {
                desc: Dirty::new(group_desc),
                block_bitmap: Dirty::new(block_bitmap),
                inode_bitmap: Dirty::new(inode_bitmap),
            }),
            block_device,
            first_block: first_block_no,
            last_block: last_block_no,
            nr_inode_table_blocks_per_group,
            nr_inodes_per_group,
            inode_size,
            _inode_table_backend: backend,
            inode_table_cache,
            inode_cache: RwMutex::new(BTreeMap::new()),
        })
    }

    /// Returns the block group index.
    pub(super) fn group_idx(&self) -> usize {
        self.group_idx
    }

    /// Returns whether an inode is marked allocated in this group.
    pub(super) fn is_inode_allocated(&self, ino: Ext4Ino) -> bool {
        let inode_idx = self.inode_idx_in_group(ino);
        self.metadata.read().inode_bitmap.is_allocated(inode_idx)
    }

    /// Looks up an allocated inode by inode number.
    pub(super) fn lookup_inode(&self, ino: Ext4Ino, fs: Weak<Ext4>) -> Result<Arc<Inode>> {
        let inode_idx = self.inode_idx_in_group(ino);

        // Fast path: cache hit under read lock.
        if let Some(inode) = self.inode_cache.read().get(&inode_idx) {
            return Ok(inode.clone());
        }

        // Slow path: check allocation bitmap, then load from disk.
        if !self.is_inode_allocated(ino) {
            return_errno!(Errno::ENOENT);
        }

        // Revalidate under write lock since another thread may have inserted
        // the inode between the read and write lock acquisition.
        let mut inode_cache = self.inode_cache.write();
        if let Some(inode) = inode_cache.get(&inode_idx) {
            return Ok(inode.clone());
        }

        let inode_desc = self.read_inode_desc(inode_idx)?;
        let filesystem = fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "filesystem already dropped"))?;
        if inode_desc.uses_extents() && !filesystem.has_extents() {
            return_errno_with_message!(
                Errno::EUCLEAN,
                "extent inode found on a filesystem without the extents feature"
            );
        }
        if filesystem.mount_flavor() == MountFlavor::Ext4 && inode_desc.is_indexed_directory() {
            return_errno_with_message!(
                Errno::EOPNOTSUPP,
                "hash-indexed directories are unsupported"
            );
        }
        let inode_desc = Dirty::new(inode_desc);
        let inode = Inode::new(
            ino,
            inode_desc.type_(),
            inode_desc,
            self.group_idx,
            Arc::downgrade(&filesystem),
        );
        inode_cache.insert(inode_idx, inode.clone());
        Ok(inode)
    }

    /// Inserts an inode into this group's in-memory cache.
    pub(super) fn insert_inode(&self, inode: Arc<Inode>) {
        let inode_idx = self.inode_idx_in_group(inode.ino());
        self.inode_cache.write().insert(inode_idx, inode);
    }

    /// Removes an inode from this group's in-memory cache.
    pub(super) fn remove_inode(&self, ino: Ext4Ino) -> Option<Arc<Inode>> {
        let inode_idx = self.inode_idx_in_group(ino);
        self.inode_cache.write().remove(&inode_idx)
    }

    /// Stages cached inode state and flushes the local inode table.
    pub(super) fn sync_inodes(&self) -> Result<()> {
        // Clone the `Arc` handles under the read lock, then drop the lock before
        // calling `sync`. Otherwise `sync` acquires `inner.write()` while
        // we still hold `inode_cache.read()`, creating a lock-order inversion with
        // `create`/`unlink`/`rmdir`/`rename` which take `inner.write()` first, then
        // `inode_cache.write()`.
        let inodes: Vec<Arc<Inode>> = self.inode_cache.read().values().cloned().collect();
        for inode in inodes {
            inode.sync_all()?;
        }
        self.sync_inode_table()
    }

    /// Syncs the inode table back to disk.
    pub(super) fn sync_inode_table(&self) -> Result<()> {
        // TODO: support sync specific inode with inode number.
        let size = self.nr_inodes_per_group as usize * self.inode_size;
        let range = 0..size;
        self.inode_table_cache.flush_range(range)
    }

    /// Flushes the page(s) containing one inode slot, including a crossing slot.
    pub(super) fn sync_inode_slot(&self, ino: Ext4Ino) -> Result<()> {
        let offset = self.inode_idx_in_group(ino) as usize * self.inode_size;
        self.inode_table_cache
            .flush_range(offset..offset + self.inode_size)
    }

    /// Returns a read guard over the combined group metadata.
    #[cfg(ktest)]
    pub(super) fn metadata(&self) -> RwMutexReadGuard<'_, BlockGroupMetadata> {
        self.metadata.read()
    }

    /// Returns the first filesystem-wide block number of this group.
    pub(super) fn first_block(&self) -> u32 {
        self.first_block
    }

    /// Returns the last filesystem-wide block number of this group.
    pub(super) fn last_block(&self) -> u32 {
        self.last_block
    }

    /// Returns the number of free blocks in this group.
    pub(super) fn free_blocks_count(&self) -> u16 {
        self.metadata.read().desc.free_blocks_count
    }

    /// Returns the number of free inodes in this group.
    pub(super) fn free_inodes_count(&self) -> u16 {
        self.metadata.read().desc.free_inodes_count
    }

    /// Returns whether the group descriptor has been modified since the last writeback.
    pub(super) fn is_desc_dirty(&self) -> bool {
        self.metadata.read().desc.is_dirty()
    }

    /// Attempts to allocate up to `count` contiguous blocks within this group.
    ///
    /// Returns `Ok(range)` with filesystem-wide block numbers on success, or an empty range
    /// if no allocatable blocks are available. Returns `Err(EIO)` on bitmap corruption.
    pub(super) fn alloc_blocks(&self, count: u32, sb_free_blocks: u32) -> Result<Range<Ext4Bid>> {
        let group_size = self.last_block - self.first_block + 1;
        debug_assert!(group_size <= IdBitmap::capacity() as u32);

        let mut metadata = self.metadata.write();

        let mut requested_count = count
            .min(group_size)
            .min(metadata.desc.free_blocks_count as u32)
            .min(sb_free_blocks) as u16;

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
            if metadata.desc.free_blocks_count > 0 {
                return_errno_with_message!(Errno::EIO, "block bitmap corruption detected");
            }
            return Ok(0..0);
        };

        let range_start = range.start as u32;
        let alloc_count = range.len() as u32;

        let abs_range =
            (self.first_block + range_start)..(self.first_block + range_start + alloc_count);
        if self.overlaps_system_zone_with(&metadata.desc, abs_range)
            || (metadata.desc.free_blocks_count as u32) < alloc_count
            || sb_free_blocks < alloc_count
        {
            metadata.block_bitmap.free_consecutive(range);
            return_errno_with_message!(Errno::EIO, "block bitmap corruption detected");
        }

        let alloc_count = alloc_count as u16;
        debug_assert!(metadata.desc.free_blocks_count >= alloc_count);
        metadata.desc.free_blocks_count -= alloc_count;

        let range_start_block = self.first_block + range.start as u32;
        let range_end_block = self.first_block + range.end as u32;
        Ok(range_start_block..range_end_block)
    }

    /// Frees a range of blocks within this group.
    ///
    /// Frees a contiguous range of group-relative block bits.
    /// Returns the number of blocks actually freed.
    pub(super) fn free_blocks(&self, bit_range: Range<u32>) -> Result<u32> {
        let start_bit = bit_range.start;
        let group_count = bit_range.len() as u32;
        // Validate system zone overlap using filesystem-wide coordinates.
        let abs_range = (self.first_block + start_bit)..(self.first_block + bit_range.end);

        let mut metadata = self.metadata.write();

        if self.overlaps_system_zone_with(&metadata.desc, abs_range) {
            return_errno_with_message!(Errno::EIO, "freeing blocks in system zone");
        }

        // Clear bits one by one and count only allocated-to-free transitions.
        let range_start = start_bit as u16;
        let range_end = (start_bit + group_count) as u16;
        let expected = (range_start..range_end)
            .filter(|&bit| metadata.block_bitmap.is_allocated(bit))
            .count() as u16;
        let new_free = metadata
            .desc
            .free_blocks_count
            .checked_add(expected)
            .ok_or_else(|| Error::with_message(Errno::EIO, "free block count overflow in group"))?;
        let mut actually_freed: u32 = 0;
        for block_bit in range_start..range_end {
            if !metadata.block_bitmap.is_allocated(block_bit) {
                warn!(
                    "free_blocks: bit already cleared for block {}",
                    self.first_block + start_bit + (block_bit - range_start) as u32
                );
            } else {
                metadata.block_bitmap.free(block_bit);
                actually_freed += 1;
            }
        }

        // Persistent in-memory bitmap cache; writeback is deferred to sync_metadata.

        metadata.desc.free_blocks_count = new_free;

        Ok(actually_freed)
    }

    /// Checks address bounds and protected metadata without scanning allocation bits.
    pub(super) fn is_data_area_range(&self, range: Range<Ext4Bid>) -> bool {
        if range.is_empty() || range.start < self.first_block || range.end > self.last_block + 1 {
            return false;
        }
        !self.overlaps_system_zone_with(&self.metadata.read().desc, range)
    }

    /// Returns whether every block in an absolute range is allocated and is
    /// outside this group's bitmap and inode-table metadata.
    #[cfg(ktest)]
    pub(super) fn is_allocated_data_range(&self, range: Range<Ext4Bid>) -> bool {
        if range.is_empty() || range.start < self.first_block || range.end > self.last_block + 1 {
            return false;
        }

        let metadata = self.metadata.read();
        if self.overlaps_system_zone_with(&metadata.desc, range.clone()) {
            return false;
        }

        (range.start..range.end).all(|block| {
            metadata
                .block_bitmap
                .is_allocated((block - self.first_block) as u16)
        })
    }

    /// Attempts to allocate one inode within this group.
    ///
    /// Returns `Some(inode_idx)` with the 0-based group-relative inode index,
    /// or `None` if no free inode is available in this group.
    pub(super) fn alloc_ino(&self, inode_type: InodeType) -> Result<Option<Ext4Ino>> {
        let mut metadata = self.metadata.write();

        if metadata.desc.free_inodes_count == 0 {
            return Ok(None);
        }
        if inode_type.is_directory() && metadata.desc.used_dirs_count == u16::MAX {
            return_errno_with_message!(Errno::EIO, "group used directory counter overflow");
        }

        let Some(inode_idx) = metadata.inode_bitmap.alloc() else {
            return Ok(None);
        };

        metadata.desc.free_inodes_count = metadata
            .desc
            .free_inodes_count
            .checked_sub(1)
            .ok_or_else(|| Error::with_message(Errno::EIO, "group free inode counter underflow"))?;
        if inode_type.is_directory() {
            metadata.desc.used_dirs_count += 1;
        }

        Ok(Some(inode_idx as Ext4Ino))
    }

    /// Frees one inode within this group.
    ///
    /// Returns `true` if the allocation state transitioned allocated-to-free,
    /// `false` if it was already free (logs warning).
    pub(super) fn free_inode(&self, ino: Ext4Ino, inode_type: InodeType) -> Result<bool> {
        let inode_idx = self.inode_idx_in_group(ino);
        let mut metadata = self.metadata.write();

        if !metadata.inode_bitmap.is_allocated(inode_idx) {
            warn!("free_inode: inode idx {} already freed", inode_idx);
            return Ok(false);
        }

        let free_inodes_count = metadata
            .desc
            .free_inodes_count
            .checked_add(1)
            .ok_or_else(|| Error::with_message(Errno::EIO, "group free inode counter overflow"))?;
        let used_dirs_count = if inode_type.is_directory() {
            Some(
                metadata
                    .desc
                    .used_dirs_count
                    .checked_sub(1)
                    .ok_or_else(|| {
                        Error::with_message(Errno::EIO, "group used directory counter underflow")
                    })?,
            )
        } else {
            None
        };

        metadata.inode_bitmap.free(inode_idx);
        metadata.desc.free_inodes_count = free_inodes_count;
        if let Some(used_dirs_count) = used_dirs_count {
            metadata.desc.used_dirs_count = used_dirs_count;
        }

        Ok(true)
    }

    /// Reads an inode descriptor from the group's inode table `PageCache`.
    ///
    /// `inode_idx` is the 0-based inode index within this group.
    pub(super) fn read_inode_desc(&self, inode_idx: u16) -> Result<InodeDesc> {
        let offset_bytes = (inode_idx as usize) * self.inode_size;
        let mut raw_inode = RawInode::default();
        self.inode_table_cache.read_bytes(
            offset_bytes,
            &mut raw_inode.as_mut_bytes()[..self.inode_size],
        )?;
        InodeDesc::try_from(&raw_inode)
    }

    /// Writes an inode descriptor to the group's inode table `PageCache`.
    pub(super) fn write_back_inode_desc(&self, ino: Ext4Ino, raw: &RawInode) -> Result<()> {
        let inode_idx = self.inode_idx_in_group(ino);
        let offset_bytes = (inode_idx as usize) * self.inode_size;
        self.inode_table_cache
            .write_bytes(offset_bytes, &raw.as_bytes()[..self.inode_size])?;
        Ok(())
    }

    /// Writes dirty bitmaps and stages the group descriptor under a single lock.
    ///
    /// Dirty bitmaps are written to disk here. If the group descriptor is dirty,
    /// this method updates the caller-provided descriptor table segment; the
    /// caller is responsible for writing that segment to disk.
    pub(super) fn sync_metadata(&self, group_descs: &USegment) -> Result<()> {
        self.metadata
            .write()
            .sync(self.block_device.as_ref(), self.group_idx, group_descs)
    }

    /// Returns the 0-based group-local inode index.
    fn inode_idx_in_group(&self, ino: Ext4Ino) -> u16 {
        debug_assert!(ino > 0);
        debug_assert_eq!(
            ((ino - 1) / self.nr_inodes_per_group) as usize,
            self.group_idx
        );
        ((ino - 1) % self.nr_inodes_per_group) as u16
    }

    /// Loads and validates the block bitmap for this group.
    fn load_block_bitmap(
        block_device: &dyn BlockDevice,
        first_block: u32,
        last_block: u32,
        desc: &BlockGroupDesc,
    ) -> Result<IdBitmap> {
        let bitmap_bid = desc.block_bitmap_bid;

        let mut buf = vec![0u8; BLOCK_SIZE];
        if block_device
            .read_bytes(Bid::new(bitmap_bid as u64).to_offset(), &mut buf)
            .is_err()
        {
            return_errno_with_message!(Errno::EIO, "failed to read block bitmap");
        }

        let max_bit = last_block - first_block;
        let capacity = (max_bit + 1) as usize;
        debug_assert!(capacity <= IdBitmap::capacity() as usize);
        Ok(IdBitmap::from_buf(buf.into_boxed_slice(), capacity as u16))
    }

    /// Loads the inode bitmap for this group.
    fn load_inode_bitmap(
        block_device: &dyn BlockDevice,
        nr_inodes_per_group: u32,
        desc: &BlockGroupDesc,
    ) -> Result<IdBitmap> {
        let bitmap_bid = desc.inode_bitmap_bid;

        let mut buf = vec![0u8; BLOCK_SIZE];
        if block_device
            .read_bytes(Bid::new(bitmap_bid as u64).to_offset(), &mut buf)
            .is_err()
        {
            return_errno_with_message!(Errno::EIO, "failed to read inode bitmap");
        }

        let capacity = nr_inodes_per_group as usize;
        debug_assert!(capacity <= IdBitmap::capacity() as usize);

        Ok(IdBitmap::from_buf(buf.into_boxed_slice(), capacity as u16))
    }

    /// Checks whether [start, start+count-1] overlaps any system metadata block
    /// (block bitmap, inode bitmap, inode table) of this group.
    ///
    /// This variant accepts a `BlockGroupDesc` reference directly, allowing callers
    /// that already hold the metadata lock to avoid re-acquiring it.
    fn overlaps_system_zone_with(&self, desc: &BlockGroupDesc, range: Range<u32>) -> bool {
        if range.is_empty() {
            return false;
        }

        let block_bitmap = desc.block_bitmap_bid..(desc.block_bitmap_bid + 1);
        let inode_bitmap = desc.inode_bitmap_bid..(desc.inode_bitmap_bid + 1);
        let inode_table =
            desc.inode_table_bid..(desc.inode_table_bid + self.nr_inode_table_blocks_per_group);

        Self::ranges_overlap(&range, &block_bitmap)
            || Self::ranges_overlap(&range, &inode_bitmap)
            || Self::ranges_overlap(&range, &inode_table)
    }

    fn ranges_overlap(a: &Range<u32>, b: &Range<u32>) -> bool {
        !a.is_empty() && !b.is_empty() && a.start < b.end && b.start < a.end
    }
}

/// Backend of the inode table page cache in one block group.
struct InodeTableBackend {
    /// Physical block ID of `bg_inode_table`.
    inode_table_bid: Ext4Bid,
    /// Total inode table size in bytes (`nr_inodes_per_group * inode_size`).
    raw_inodes_size: usize,
    /// Block device handle for I/O.
    block_device: Arc<dyn BlockDevice>,
}

impl BlockAsPageCacheBackend for InodeTableBackend {
    fn submit_read_bio(
        &self,
        idx: usize,
        bio_segment: BioSegment,
        complete_fn: BioCompleteFn,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        if self.raw_inodes_size <= idx * BLOCK_SIZE {
            return_errno_with_message!(Errno::EINVAL, "invalid read size");
        }
        let bid = Bid::new(self.inode_table_bid as u64) + idx as u64;
        self.block_device
            .read_blocks_async(bid, vec![bio_segment], Some(complete_fn), io_batch)?;
        Ok(())
    }

    fn submit_read_page_run(&self, mut pages: PageRun<'_>, io_batch: &mut IoBatch) -> Result<()> {
        if self.raw_inodes_size < (pages.start_idx() + pages.len()) * BLOCK_SIZE {
            return_errno_with_message!(Errno::EINVAL, "invalid read size");
        }

        let start_idx = pages.start_idx();
        let bid = Bid::new(self.inode_table_bid as u64) + start_idx as u64;
        let len = pages.len();

        pages.submit_contiguous_read_pages(self.block_device.as_ref(), bid, len, io_batch)
    }

    fn submit_write_bio(
        &self,
        idx: usize,
        bio_segment: BioSegment,
        complete_fn: BioCompleteFn,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        if self.raw_inodes_size <= idx * BLOCK_SIZE {
            return_errno_with_message!(Errno::EINVAL, "invalid write size");
        }
        let bid = Bid::new(self.inode_table_bid as u64) + idx as u64;
        self.block_device.write_blocks_async(
            bid,
            vec![bio_segment],
            Some(complete_fn),
            io_batch,
        )?;
        Ok(())
    }

    fn submit_write_page_run(&self, mut pages: PageRun<'_>, io_batch: &mut IoBatch) -> Result<()> {
        if self.raw_inodes_size < (pages.start_idx() + pages.len()) * BLOCK_SIZE {
            return_errno_with_message!(Errno::EINVAL, "invalid write size");
        }

        let start_idx = pages.start_idx();
        let bid = Bid::new(self.inode_table_bid as u64) + start_idx as u64;
        let len = pages.len();

        pages.submit_contiguous_write_pages(self.block_device.as_ref(), bid, len, io_batch)
    }
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::ktest;

    use super::*;
    use crate::fs::fs_impls::ext4::test_utils::{Ext4FixtureBuilder, RawInodeBuilder};

    #[ktest]
    fn allocation_metadata_retries_only_unwritten_bitmaps() {
        let fixture = Ext4FixtureBuilder::new(1, 2048)
            .with_free_blocks(8, 8)
            .with_free_inodes(16, 16)
            .build()
            .unwrap();
        let group = fixture.fs.block_group(0);
        assert!(!group.alloc_blocks(1, 8).unwrap().is_empty());
        assert!(group.alloc_ino(InodeType::Dir).unwrap().is_some());
        let (block_bid, inode_bid, expected) = {
            let metadata = group.metadata.read();
            (
                metadata.desc.block_bitmap_bid,
                metadata.desc.inode_bitmap_bid,
                RawBlockGroup::from(*metadata.desc),
            )
        };
        let descriptors: USegment = FrameAllocOptions::new()
            .zeroed(true)
            .alloc_segment(1)
            .unwrap()
            .into();
        fixture.disk.fail_write_block(Some(block_bid as usize));
        assert!(group.sync_metadata(&descriptors).is_err());
        assert!(group.metadata.read().block_bitmap.is_dirty());
        assert!(group.metadata.read().inode_bitmap.is_dirty());
        assert_eq!(fixture.disk.block_writes(inode_bid as usize), 0);

        fixture.disk.fail_write_block(Some(inode_bid as usize));
        assert!(group.sync_metadata(&descriptors).is_err());
        assert!(!group.metadata.read().block_bitmap.is_dirty());
        assert!(group.metadata.read().inode_bitmap.is_dirty());
        assert!(group.is_desc_dirty());
        let block_writes = fixture.disk.block_writes(block_bid as usize);

        fixture.disk.fail_write_block(None);
        group.sync_metadata(&descriptors).unwrap();
        assert_eq!(fixture.disk.block_writes(block_bid as usize), block_writes);
        assert!(!group.metadata.read().inode_bitmap.is_dirty());
        assert!(!group.is_desc_dirty());
        let staged = descriptors.read_val::<RawBlockGroup>(0).unwrap();
        assert_eq!(staged.as_bytes(), expected.as_bytes());
        // Descriptor staging is deliberately separate from device writeback.
        assert_eq!(
            fixture
                .disk
                .block_writes(fixture.sb.group_descriptors_bid(0) as usize),
            0
        );
    }

    #[ktest]
    fn deferred_extent_release_does_not_retry_completed_group() {
        use crate::fs::ext4::reclaim::DeferredReclaim;
        let f = Ext4FixtureBuilder::new(2, 256)
            .with_blocks_per_group(128)
            .with_free_blocks(100, 50)
            .build()
            .unwrap();
        let a = f.fs.block_group(0).alloc_blocks(1, 100).unwrap();
        f.fs.block_group(1).metadata.write().desc.free_blocks_count = 50;
        let b = f.fs.block_group(1).alloc_blocks(1, 100).unwrap();
        assert!(!a.is_empty() && !b.is_empty());
        let group = f.fs.block_group(1);
        let saved = group.metadata.read().desc.free_blocks_count;
        group.metadata.write().desc.free_blocks_count = u16::MAX;
        assert!(DeferredReclaim::release(&f.fs, vec![a.clone(), b.clone()]).is_err());
        let reused = f.fs.alloc_blocks(1, a.start).unwrap();
        assert_eq!(reused, a);
        group.metadata.write().desc.free_blocks_count = saved;
        f.fs.retry_extent_work().unwrap();
        assert!(f.fs.block_group(0).is_allocated_data_range(a));
        assert!(!group.is_allocated_data_range(b));
    }

    #[ktest]
    fn inode_slot_read_update_write_preserves_layout_and_unknown_bytes() {
        for inode_size in [128, 256] {
            const INO: Ext4Ino = 12;

            let fixture = Ext4FixtureBuilder::new(1, 256)
                .with_inode_size(inode_size)
                .with_inodes_per_group(32)
                .build()
                .unwrap();
            assert_eq!(fixture.sb.inode_size(), inode_size);

            let group = fixture.fs.block_group(0);
            let inode_idx = (INO - 1) as usize;
            let inode_offset = inode_idx * inode_size;

            let mut first_raw = RawInodeBuilder::new(0o100644).size_lo(17).build();
            first_raw.osd1 = 0x1122_3344;
            first_raw.osd2_reserved = 0x5566;
            if inode_size == 256 {
                first_raw.as_mut_bytes()[160..].fill(0x5A);
            }
            let first_slot = first_raw.as_bytes()[..inode_size].to_vec();
            let second_slot = vec![0xA5; inode_size];

            group
                .inode_table_cache
                .write_bytes(inode_offset, &first_slot)
                .unwrap();
            group
                .inode_table_cache
                .write_bytes(inode_offset + inode_size, &second_slot)
                .unwrap();

            let desc = group.read_inode_desc(inode_idx as u16).unwrap();
            let mut updated = RawInode::from(&desc);
            updated.size_lo = 4096;
            group.write_back_inode_desc(INO, &updated).unwrap();

            let mut actual = vec![0u8; inode_size * 2];
            group
                .inode_table_cache
                .read_bytes(inode_offset, &mut actual)
                .unwrap();

            let mut expected_first = first_slot;
            expected_first[4..8].copy_from_slice(&4096u32.to_ne_bytes());
            assert_eq!(&actual[..inode_size], expected_first.as_slice());
            assert_eq!(&actual[inode_size..], &second_slot);
            let reloaded = group.read_inode_desc(inode_idx as u16).unwrap();
            assert_eq!(RawInode::from(&reloaded).size_lo, 4096);
        }
    }
}
