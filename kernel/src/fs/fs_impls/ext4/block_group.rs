// SPDX-License-Identifier: MPL-2.0

//! Block group and group descriptor for Ext4.

use super::{
    inode::Inode as Ext4Inode,
    prelude::*,
    super_block::SuperBlock,
};

/// The in-memory rust block group descriptor.
#[derive(Clone, Copy, Debug)]
pub(super) struct GroupDescriptor {
    pub block_bitmap_bid: u64,
    pub inode_bitmap_bid: u64,
    pub inode_table_bid: u64,
    pub free_blocks_count: u32,
    pub free_inodes_count: u32,
    pub dirs_count: u16,
    pub flags: u16,
}

/// The raw ext4 group descriptor on disk (64 bytes with 64-bit support).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct RawGroupDescriptor {
    pub block_bitmap_lo: u32,
    pub inode_bitmap_lo: u32,
    pub inode_table_lo: u32,
    pub free_blocks_count_lo: u16,
    pub free_inodes_count_lo: u16,
    pub dirs_count_lo: u16,
    pub flags: u16,
    pub exclude_bitmap_lo: u32,
    pub block_bitmap_csum_lo: u16,
    pub inode_bitmap_csum_lo: u16,
    pub itable_unused_lo: u16,
    pub checksum: u16,
    // 64-bit extensions
    pub block_bitmap_hi: u32,
    pub inode_bitmap_hi: u32,
    pub inode_table_hi: u32,
    pub free_blocks_count_hi: u16,
    pub free_inodes_count_hi: u16,
    pub dirs_count_hi: u16,
    pub itable_unused_hi: u16,
    pub checksum_hi: u32,
    pub reserved: u32,
}

impl Default for RawGroupDescriptor {
    fn default() -> Self {
        Self {
            block_bitmap_lo: 0,
            inode_bitmap_lo: 0,
            inode_table_lo: 0,
            free_blocks_count_lo: 0,
            free_inodes_count_lo: 0,
            dirs_count_lo: 0,
            flags: 0,
            exclude_bitmap_lo: 0,
            block_bitmap_csum_lo: 0,
            inode_bitmap_csum_lo: 0,
            itable_unused_lo: 0,
            checksum: 0,
            block_bitmap_hi: 0,
            inode_bitmap_hi: 0,
            inode_table_hi: 0,
            free_blocks_count_hi: 0,
            free_inodes_count_hi: 0,
            dirs_count_hi: 0,
            itable_unused_hi: 0,
            checksum_hi: 0,
            reserved: 0,
        }
    }
}

impl From<RawGroupDescriptor> for GroupDescriptor {
    fn from(desc: RawGroupDescriptor) -> Self {
        Self {
            block_bitmap_bid: ((desc.block_bitmap_hi as u64) << 32) | desc.block_bitmap_lo as u64,
            inode_bitmap_bid: ((desc.inode_bitmap_hi as u64) << 32) | desc.inode_bitmap_lo as u64,
            inode_table_bid: ((desc.inode_table_hi as u64) << 32) | desc.inode_table_lo as u64,
            free_blocks_count: ((desc.free_blocks_count_hi as u32) << 16) | desc.free_blocks_count_lo as u32,
            free_inodes_count: ((desc.free_inodes_count_hi as u32) << 16) | desc.free_inodes_count_lo as u32,
            dirs_count: desc.dirs_count_lo,
            flags: desc.flags,
        }
    }
}

/// A block group in Ext4.
#[derive(Debug)]
pub(super) struct BlockGroup {
    idx: usize,
    bg_impl: Arc<BlockGroupImpl>,
    raw_inodes_cache: PageCache,
}

struct BlockGroupImpl {
    inode_table_bid: u64,
    raw_inodes_size: usize,
    inner: RwMutex<Inner>,
    fs: Weak<super::fs::Ext4>,
}

#[derive(Debug)]
struct Inner {
    descriptor: GroupDescriptor,
    inode_cache: BTreeMap<u32, Arc<Ext4Inode>>,
    inode_bitmap: crate::fs::utils::IdBitmap,
}

impl BlockGroup {
    pub fn load(
        group_descriptors_segment: &USegment,
        idx: usize,
        block_device: &dyn BlockDevice,
        super_block: &SuperBlock,
        fs: Weak<super::fs::Ext4>,
    ) -> Result<Self> {
        let raw_inodes_size = (super_block.inodes_per_group() as usize) * super_block.inode_size();
        let desc_size = super_block.desc_size();

        let bg_impl = {
            let descriptor = {
                let offset = idx * desc_size;
                let raw_descriptor = group_descriptors_segment
                    .read_val::<RawGroupDescriptor>(offset)
                    .map_err(|_| Error::with_message(Errno::EIO, "failed to read group descriptor"))?;
                GroupDescriptor::from(raw_descriptor)
            };

            let inode_bitmap = {
                let capacity = super_block.inodes_per_group() as usize;
                if capacity > super_block.block_size() * 8 {
                    return_errno_with_message!(Errno::EINVAL, "bad inode bitmap capacity");
                }
                let mut buf = alloc::vec![0u8; super_block.block_size()];
                let offset = descriptor.inode_bitmap_bid as usize * super_block.block_size();
                block_device.read_bytes(offset, &mut buf)?;
                crate::fs::utils::IdBitmap::from_buf(buf.into_boxed_slice(), capacity as u16)
            };

            Arc::new(BlockGroupImpl {
                inode_table_bid: descriptor.inode_table_bid,
                raw_inodes_size,
                inner: RwMutex::new(Inner {
                    descriptor,
                    inode_cache: BTreeMap::new(),
                    inode_bitmap,
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

    /// Finds and returns the inode by its index within this group.
    pub fn lookup_inode(&self, inode_idx: u32) -> Result<Arc<Ext4Inode>> {
        let inner = self.bg_impl.inner.read();
        if !inner.inode_bitmap.is_allocated(inode_idx as u16) {
            return_errno!(Errno::ENOENT);
        }
        if let Some(inode) = inner.inode_cache.get(&inode_idx) {
            return Ok(inode.clone());
        }
        drop(inner);

        let mut inner = self.bg_impl.inner.write();
        if !inner.inode_bitmap.is_allocated(inode_idx as u16) {
            return_errno!(Errno::ENOENT);
        }
        if let Some(inode) = inner.inode_cache.get(&inode_idx) {
            return Ok(inode.clone());
        }

        let inode = self.load_inode(inode_idx)?;
        inner.inode_cache.insert(inode_idx, inode.clone());
        Ok(inode)
    }

    fn load_inode(&self, inode_idx: u32) -> Result<Arc<Ext4Inode>> {
        let fs = self.fs();
        let raw_inode = {
            let offset = (inode_idx as usize) * fs.inode_size();
            self.raw_inodes_cache
                .pages()
                .read_val::<super::inode::RawInode>(offset)
                .map_err(|_| Error::with_message(Errno::EIO, "failed to read raw inode"))?
        };
        let inode_desc = super::inode::InodeDesc::try_from(raw_inode)?;
        let ino = inode_idx + self.idx as u32 * fs.inodes_per_group() + 1;
        Ok(Ext4Inode::new(ino, self.idx, inode_desc, Arc::downgrade(&fs)))
    }

    fn fs(&self) -> Arc<super::fs::Ext4> {
        self.bg_impl.fs.upgrade().unwrap()
    }
}

impl PageCacheBackend for BlockGroupImpl {
    fn read_page_async(&self, idx: usize, frame: &CachePage) -> Result<BioWaiter> {
        let bid = self.inode_table_bid + idx as u64;
        let bio_segment = BioSegment::new_from_segment(
            Segment::from(frame.clone()).into(),
            BioDirection::FromDevice,
        );
        self.fs.upgrade().unwrap().read_blocks_async(bid, bio_segment)
    }

    fn write_page_async(&self, _idx: usize, _frame: &CachePage) -> Result<BioWaiter> {
        Err(Error::with_message(Errno::EROFS, "read-only filesystem"))
    }

    fn npages(&self) -> usize {
        self.raw_inodes_size.div_ceil(BLOCK_SIZE)
    }
}
