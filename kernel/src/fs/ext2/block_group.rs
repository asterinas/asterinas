// SPDX-License-Identifier: MPL-2.0

use id_alloc::IdAlloc;

use super::{
    block_ptr::Ext2Bid,
    fs::Ext2,
    inode::{Inode, InodeDesc, RawInode},
    prelude::*,
    super_block::SuperBlock,
};

/// Blocks are clustered into block groups in order to reduce fragmentation and minimise
/// the amount of head seeking when reading a large amount of consecutive data.
pub(super) struct BlockGroup {
    idx: usize,
    bg_impl: Arc<BlockGroupImpl>,
    raw_inodes_cache: PageCache,
}

struct BlockGroupImpl {
    inode_table_bid: Ext2Bid,
    raw_inodes_size: usize,
    inner: RwMutex<Inner>,
    fs: Weak<Ext2>,
}

impl BlockGroup {
    /// Loads and constructs a block group.
    pub fn load(
        group_descriptors_segment: &Segment,
        idx: usize,
        block_device: &dyn BlockDevice,
        super_block: &SuperBlock,
        fs: Weak<Ext2>,
    ) -> Result<Self> {
        let raw_inodes_size = (super_block.inodes_per_group() as usize) * super_block.inode_size();

        let bg_impl = {
            let metadata = {
                let descriptor = {
                    // Read the block group descriptor
                    // TODO: if the main is corrupted, should we load the backup?
                    let offset = idx * core::mem::size_of::<RawGroupDescriptor>();
                    let raw_descriptor = group_descriptors_segment
                        .read_val::<RawGroupDescriptor>(offset)
                        .unwrap();
                    GroupDescriptor::from(raw_descriptor)
                };

                let get_bitmap = |bid: Ext2Bid, capacity: usize| -> Result<IdAlloc> {
                    if capacity > BLOCK_SIZE * 8 {
                        return_errno_with_message!(Errno::EINVAL, "bad bitmap");
                    }
                    let mut buf = vec![0u8; BLOCK_SIZE];
                    block_device.read_bytes(bid as usize * BLOCK_SIZE, &mut buf)?;
                    Ok(IdAlloc::from_bytes_with_capacity(&buf, capacity))
                };

                let block_bitmap = get_bitmap(
                    descriptor.block_bitmap_bid,
                    super_block.blocks_per_group() as usize,
                )?;
                let inode_bitmap = get_bitmap(
                    descriptor.inode_bitmap_bid,
                    super_block.inodes_per_group() as usize,
                )?;

                GroupMetadata {
                    descriptor,
                    block_bitmap,
                    inode_bitmap,
                }
            };

            Arc::new(BlockGroupImpl {
                inode_table_bid: metadata.descriptor.inode_table_bid,
                raw_inodes_size,
                inner: RwMutex::new(Inner {
                    metadata: Dirty::new(metadata),
                    inode_cache: BTreeMap::new(),
                }),
                fs,
            })
        };

        let raw_inodes_cache =
            PageCache::with_capacity(raw_inodes_size, Arc::downgrade(&bg_impl) as _)?;

        Ok(Self {
            idx,
            bg_impl,
            raw_inodes_cache,
        })
    }

    /// Finds and returns the inode.
    pub fn lookup_inode(&self, inode_idx: u32) -> Result<Arc<Inode>> {
        // The fast path
        let inner = self.bg_impl.inner.read();
        if !inner.metadata.is_inode_allocated(inode_idx) {
            return_errno!(Errno::ENOENT);
        }
        if let Some(inode) = inner.inode_cache.get(&inode_idx) {
            return Ok(inode.clone());
        }

        // The slow path
        drop(inner);
        let mut inner = self.bg_impl.inner.write();
        if !inner.metadata.is_inode_allocated(inode_idx) {
            return_errno!(Errno::ENOENT);
        }
        if let Some(inode) = inner.inode_cache.get(&inode_idx) {
            return Ok(inode.clone());
        }

        // Loads the inode, then inserts it into the inode cache.
        let inode = self.load_inode(inode_idx)?;
        inner.inode_cache.insert(inode_idx, inode.clone());
        Ok(inode)
    }

    /// Loads an existing inode.
    ///
    /// This method may load the raw inode metadata from block device.
    fn load_inode(&self, inode_idx: u32) -> Result<Arc<Inode>> {
        let fs = self.fs();
        let raw_inode = {
            let offset = (inode_idx as usize) * fs.inode_size();
            self.raw_inodes_cache
                .pages()
                .read_val::<RawInode>(offset)
                .unwrap()
        };
        let inode_desc = Dirty::new(InodeDesc::try_from(raw_inode)?);
        let ino = inode_idx + self.idx as u32 * fs.inodes_per_group() + 1;

        Ok(Inode::new(ino, self.idx, inode_desc, Arc::downgrade(&fs)))
    }

    /// Inserts the inode into the inode cache.
    ///
    /// # Panics
    ///
    /// If `inode_idx` has not been allocated before, then the method panics.
    pub fn insert_cache(&self, inode_idx: u32, inode: Arc<Inode>) {
        let mut inner = self.bg_impl.inner.write();
        assert!(inner.metadata.is_inode_allocated(inode_idx));
        inner.inode_cache.insert(inode_idx, inode);
    }

    /// Allocates and returns an inode index.
    pub fn alloc_inode(&self, is_dir: bool) -> Option<u32> {
        // The fast path
        if self.bg_impl.inner.read().metadata.free_inodes_count() == 0 {
            return None;
        }

        // The slow path
        self.bg_impl.inner.write().metadata.alloc_inode(is_dir)
    }

    /// Frees the allocated inode idx.
    ///
    /// # Panics
    ///
    /// If `inode_idx` has not been allocated before, then the method panics.
    pub fn free_inode(&self, inode_idx: u32, is_dir: bool) {
        let mut inner = self.bg_impl.inner.write();
        assert!(inner.metadata.is_inode_allocated(inode_idx));

        inner.metadata.free_inode(inode_idx, is_dir);
        inner.inode_cache.remove(&inode_idx);
    }

    /// Allocates and returns a consecutive range of block indices.
    ///
    /// Returns `None` if the allocation fails.
    ///
    /// The actual allocated range size may be smaller than the requested `count` if
    /// insufficient consecutive blocks are available.
    pub fn alloc_blocks(&self, count: Ext2Bid) -> Option<Range<Ext2Bid>> {
        // The fast path
        if self.bg_impl.inner.read().metadata.free_blocks_count() == 0 {
            return None;
        }

        // The slow path
        self.bg_impl.inner.write().metadata.alloc_blocks(count)
    }

    /// Frees the consecutive range of allocated block indices.
    ///
    /// # Panics
    ///
    ///  If the `range` is out of bounds, this method will panic.
    ///  If one of the `idx` in `range` has not been allocated before, then the method panics.
    pub fn free_blocks(&self, range: Range<Ext2Bid>) {
        if range.is_empty() {
            return;
        }

        let mut inner = self.bg_impl.inner.write();
        for idx in range.clone() {
            assert!(inner.metadata.is_block_allocated(idx));
        }
        inner.metadata.free_blocks(range);
    }

    /// Writes back the raw inode metadata to the raw inode metadata cache.
    pub fn sync_raw_inode(&self, inode_idx: u32, raw_inode: &RawInode) {
        let offset = (inode_idx as usize) * self.fs().inode_size();
        self.raw_inodes_cache
            .pages()
            .write_val(offset, raw_inode)
            .unwrap();
    }

    /// Writes back the metadata of this group.
    pub fn sync_metadata(&self) -> Result<()> {
        if !self.bg_impl.inner.read().metadata.is_dirty() {
            return Ok(());
        }

        let mut inner = self.bg_impl.inner.write();
        let fs = self.fs();
        // Writes back the descriptor.
        let raw_descriptor = RawGroupDescriptor::from(&inner.metadata.descriptor);
        self.fs().sync_group_descriptor(self.idx, &raw_descriptor)?;

        let mut bio_waiter = BioWaiter::new();
        // Writes back the inode bitmap.
        let inode_bitmap_bid = Bid::new(inner.metadata.descriptor.inode_bitmap_bid as u64);
        bio_waiter.concat(fs.block_device().write_bytes_async(
            inode_bitmap_bid.to_offset(),
            inner.metadata.inode_bitmap.as_bytes(),
        )?);

        // Writes back the block bitmap.
        let block_bitmap_bid = Bid::new(inner.metadata.descriptor.block_bitmap_bid as u64);
        bio_waiter.concat(fs.block_device().write_bytes_async(
            block_bitmap_bid.to_offset(),
            inner.metadata.block_bitmap.as_bytes(),
        )?);

        // Waits for the completion of all submitted bios.
        bio_waiter.wait().ok_or_else(|| {
            Error::with_message(Errno::EIO, "failed to sync metadata of block group")
        })?;

        inner.metadata.clear_dirty();
        Ok(())
    }

    /// Writes back all of the cached inodes.
    ///
    /// The `sync_all` method of inode may modify the data of this block group,
    /// so we should not hold the lock while syncing the inodes.
    pub fn sync_all_inodes(&self) -> Result<()> {
        // Removes the inodes that is unused from the inode cache.
        let unused_inodes: Vec<Arc<Inode>> = self
            .bg_impl
            .inner
            .write()
            .inode_cache
            .extract_if(|_, inode| Arc::strong_count(inode) == 1)
            .map(|(_, inode)| inode)
            .collect();

        // Writes back the unused inodes.
        for inode in unused_inodes.iter() {
            inode.sync_all()?;
        }
        drop(unused_inodes);

        // Writes back the remaining inodes in the inode cache.
        let remaining_inodes: Vec<Arc<Inode>> = self
            .bg_impl
            .inner
            .read()
            .inode_cache
            .values()
            .cloned()
            .collect();
        for inode in remaining_inodes.iter() {
            inode.sync_all()?;
        }
        drop(remaining_inodes);

        // Writes back the raw inode metadata.
        self.raw_inodes_cache
            .pages()
            .decommit(0..self.bg_impl.raw_inodes_size)?;
        Ok(())
    }

    fn fs(&self) -> Arc<Ext2> {
        self.bg_impl.fs.upgrade().unwrap()
    }
}

impl Debug for BlockGroup {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("BlockGroup")
            .field("idx", &self.idx)
            .field("descriptor", &self.bg_impl.inner.read().metadata.descriptor)
            .field(
                "block_bitmap",
                &self.bg_impl.inner.read().metadata.block_bitmap,
            )
            .field(
                "inode_bitmap",
                &self.bg_impl.inner.read().metadata.inode_bitmap,
            )
            .finish()
    }
}

impl PageCacheBackend for BlockGroupImpl {
    fn read_page_async(&self, idx: usize, frame: &Frame) -> Result<BioWaiter> {
        let bid = self.inode_table_bid + idx as Ext2Bid;
        let bio_segment =
            BioSegment::new_from_segment(frame.clone().into(), BioDirection::FromDevice);
        self.fs
            .upgrade()
            .unwrap()
            .read_blocks_async(bid, bio_segment)
    }

    fn write_page_async(&self, idx: usize, frame: &Frame) -> Result<BioWaiter> {
        let bid = self.inode_table_bid + idx as Ext2Bid;
        let bio_segment =
            BioSegment::new_from_segment(frame.clone().into(), BioDirection::ToDevice);
        self.fs
            .upgrade()
            .unwrap()
            .write_blocks_async(bid, bio_segment)
    }

    fn npages(&self) -> usize {
        self.raw_inodes_size.div_ceil(BLOCK_SIZE)
    }
}

#[derive(Debug)]
struct Inner {
    metadata: Dirty<GroupMetadata>,
    inode_cache: BTreeMap<u32, Arc<Inode>>,
}

#[derive(Clone, Debug)]
struct GroupMetadata {
    descriptor: GroupDescriptor,
    block_bitmap: IdAlloc,
    inode_bitmap: IdAlloc,
}

impl GroupMetadata {
    pub fn is_inode_allocated(&self, inode_idx: u32) -> bool {
        self.inode_bitmap.is_allocated(inode_idx as usize)
    }

    pub fn alloc_inode(&mut self, is_dir: bool) -> Option<u32> {
        let inode_idx = self.inode_bitmap.alloc()?;
        self.dec_free_inodes();
        if is_dir {
            self.inc_dirs();
        }
        Some(inode_idx as u32)
    }

    pub fn free_inode(&mut self, inode_idx: u32, is_dir: bool) {
        self.inode_bitmap.free(inode_idx as usize);
        self.inc_free_inodes();
        if is_dir {
            self.dec_dirs();
        }
    }

    pub fn is_block_allocated(&self, block_idx: Ext2Bid) -> bool {
        self.block_bitmap.is_allocated(block_idx as usize)
    }

    pub fn alloc_blocks(&mut self, count: Ext2Bid) -> Option<Range<Ext2Bid>> {
        let mut current_count = count.min(self.free_blocks_count() as Ext2Bid) as usize;
        while current_count > 0 {
            let Some(range) = self.block_bitmap.alloc_consecutive(current_count) else {
                // It is efficient to halve the value
                current_count /= 2;
                continue;
            };
            self.dec_free_blocks(current_count as u16);
            return Some((range.start as Ext2Bid)..(range.end as Ext2Bid));
        }
        None
    }

    pub fn free_blocks(&mut self, range: Range<Ext2Bid>) {
        self.block_bitmap
            .free_consecutive((range.start as usize)..(range.end as usize));
        self.inc_free_blocks(range.len() as u16);
    }

    pub fn free_inodes_count(&self) -> u16 {
        self.descriptor.free_inodes_count
    }

    pub fn free_blocks_count(&self) -> u16 {
        self.descriptor.free_blocks_count
    }

    pub fn inc_free_inodes(&mut self) {
        self.descriptor.free_inodes_count += 1;
    }

    pub fn dec_free_inodes(&mut self) {
        debug_assert!(self.descriptor.free_inodes_count > 0);
        self.descriptor.free_inodes_count -= 1;
    }

    pub fn inc_free_blocks(&mut self, count: u16) {
        self.descriptor.free_blocks_count = self
            .descriptor
            .free_blocks_count
            .checked_add(count)
            .unwrap();
    }

    pub fn dec_free_blocks(&mut self, count: u16) {
        self.descriptor.free_blocks_count = self
            .descriptor
            .free_blocks_count
            .checked_sub(count)
            .unwrap();
    }

    pub fn inc_dirs(&mut self) {
        self.descriptor.dirs_count += 1;
    }

    pub fn dec_dirs(&mut self) {
        debug_assert!(self.descriptor.dirs_count > 0);
        self.descriptor.dirs_count -= 1;
    }
}

/// The in-memory rust block group descriptor.
///
/// The block group descriptor contains information regarding where important data
/// structures for that group are located.
#[derive(Clone, Copy, Debug)]
struct GroupDescriptor {
    /// Blocks usage bitmap block
    block_bitmap_bid: Ext2Bid,
    /// Inodes usage bitmap block
    inode_bitmap_bid: Ext2Bid,
    /// Starting block of inode table
    inode_table_bid: Ext2Bid,
    /// Number of free blocks in group
    free_blocks_count: u16,
    /// Number of free inodes in group
    free_inodes_count: u16,
    /// Number of directories in group
    dirs_count: u16,
}

impl From<RawGroupDescriptor> for GroupDescriptor {
    fn from(desc: RawGroupDescriptor) -> Self {
        Self {
            block_bitmap_bid: desc.block_bitmap,
            inode_bitmap_bid: desc.inode_bitmap,
            inode_table_bid: desc.inode_table,
            free_blocks_count: desc.free_blocks_count,
            free_inodes_count: desc.free_inodes_count,
            dirs_count: desc.dirs_count,
        }
    }
}

const_assert!(core::mem::size_of::<RawGroupDescriptor>() == 32);

/// The raw block group descriptor.
///
/// The table starts on the first block following the superblock.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct RawGroupDescriptor {
    pub block_bitmap: u32,
    pub inode_bitmap: u32,
    pub inode_table: u32,
    pub free_blocks_count: u16,
    pub free_inodes_count: u16,
    pub dirs_count: u16,
    pad: u16,
    reserved: [u32; 3],
}

impl From<&GroupDescriptor> for RawGroupDescriptor {
    fn from(desc: &GroupDescriptor) -> Self {
        Self {
            block_bitmap: desc.block_bitmap_bid,
            inode_bitmap: desc.inode_bitmap_bid,
            inode_table: desc.inode_table_bid,
            free_blocks_count: desc.free_blocks_count,
            free_inodes_count: desc.free_inodes_count,
            dirs_count: desc.dirs_count,
            pad: 0u16,
            reserved: [0u32; 3],
        }
    }
}
