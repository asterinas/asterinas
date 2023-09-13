use crate::bitmap::BitMap;
use crate::inode::Ext2Inode;
use crate::prelude::*;
use crate::super_block::Ext2SuperBlock;

/// Blocks are clustered into block groups in order to reduce fragmentation and minimise
/// the amount of head seeking when reading a large amount of consecutive data.
pub struct BlockGroup {
    idx: usize,
    inner: RwLock<Dirty<DiskBlockGroup>>,
    inode_cache: RwLock<BTreeMap<u32, Weak<Ext2Inode>>>,
}

#[derive(Clone, Debug)]
struct DiskBlockGroup {
    descriptor: BlockGroupDescriptor,
    block_bitmap: BitMap,
    inode_bitmap: BitMap,
}

impl BlockGroup {
    pub fn new(
        idx: usize,
        block_device: &dyn BlockDevice,
        super_block: &Ext2SuperBlock,
    ) -> Result<Self> {
        let descriptor = {
            let offset = super_block.block_group_descriptors_bid().to_offset()
                + idx * core::mem::size_of::<BlockGroupDescriptor>();
            block_device.read_val::<BlockGroupDescriptor>(offset)?
        };

        let get_bitmap = |bid: BlockId, bit_len: usize| -> Result<BitMap> {
            let mut buf = vec![0u8; BLOCK_SIZE];
            let mut block = BioBuf::from_slice_mut(&mut buf);
            block_device.read_block(bid, &mut block)?;
            BitMap::from_bytes_with_bit_len(block.as_slice(), bit_len)
        };

        let block_bitmap = get_bitmap(
            BlockId::new(descriptor.block_bitmap),
            super_block.blocks_per_group() as usize,
        )?;
        let inode_bitmap = get_bitmap(
            BlockId::new(descriptor.inode_bitmap),
            super_block.inodes_per_group() as usize,
        )?;

        let inner = DiskBlockGroup {
            descriptor,
            block_bitmap,
            inode_bitmap,
        };
        Ok(Self {
            idx,
            inner: RwLock::new(Dirty::new(inner)),
            inode_cache: RwLock::new(BTreeMap::new()),
        })
    }

    pub fn get_inode(&self, inode_idx: u32) -> Result<Option<Arc<Ext2Inode>>> {
        if !self
            .inner
            .read()
            .inode_bitmap
            .is_allocated(inode_idx as usize)
        {
            return Err(Error::NotFound);
        }
        let inode_option = self
            .inode_cache
            .read()
            .get(&inode_idx)
            .and_then(|weak| weak.upgrade());
        Ok(inode_option)
    }

    pub fn put_inode(&self, inode_idx: u32, inode: Weak<Ext2Inode>) {
        debug_assert!(self
            .inner
            .read()
            .inode_bitmap
            .is_allocated(inode_idx as usize));
        self.inode_cache.write().insert(inode_idx, inode);
    }

    pub fn alloc_inode(&self, is_dir: bool) -> Option<u32> {
        let mut inner = self.inner.write();
        let Some(inode_idx) = inner.inode_bitmap.alloc() else {
            return None;
        };
        inner.descriptor.free_inodes_count -= 1;
        if is_dir {
            inner.descriptor.dirs_count += 1;
        }
        Some(inode_idx as u32)
    }

    pub fn free_inode(&self, inode_idx: u32, is_dir: bool) {
        let mut inner = self.inner.write();
        debug_assert!(inner.inode_bitmap.is_allocated(inode_idx as usize));
        inner.inode_bitmap.free(inode_idx as usize);
        inner.descriptor.free_inodes_count += 1;
        if is_dir {
            inner.descriptor.dirs_count -= 1;
        }
    }

    pub fn alloc_block(&self) -> Option<u32> {
        let mut inner = self.inner.write();
        let Some(block_idx) = inner.block_bitmap.alloc() else {
            return None;
        };
        inner.descriptor.free_blocks_count -= 1;
        Some(block_idx as u32)
    }

    pub fn free_block(&self, block_idx: u32) {
        let mut inner = self.inner.write();
        inner.block_bitmap.free(block_idx as usize);
        inner.descriptor.free_blocks_count += 1;
    }

    pub fn inode_table_bid(&self) -> BlockId {
        BlockId::new(self.inner.read().descriptor.inode_table)
    }

    pub fn sync_metadata(
        &self,
        block_device: &dyn BlockDevice,
        super_block: &Ext2SuperBlock,
    ) -> Result<()> {
        if !self.inner.read().is_dirty() {
            return Ok(());
        }

        let mut inner = self.inner.write();
        // Write BlockGroupDescriptor
        let block_group_descriptor_offset = super_block.block_group_descriptors_bid().to_offset()
            + self.idx * core::mem::size_of::<BlockGroupDescriptor>();
        block_device.write_val(block_group_descriptor_offset, &inner.descriptor)?;

        // Write BlockGroupDescriptor backups
        let mut super_block_backup = *super_block;
        for idx in 1..super_block.block_groups_count() {
            if super_block.is_backup_block_group(idx) {
                super_block_backup.block_group = idx as u16;
                let block_group_descriptor_offset =
                    super_block_backup.block_group_descriptors_bid().to_offset()
                        + self.idx * core::mem::size_of::<BlockGroupDescriptor>();
                block_device.write_val(block_group_descriptor_offset, &inner.descriptor)?;
            }
        }

        // Write inode bitmap
        let inode_bitmap_offset = BlockId::new(inner.descriptor.inode_bitmap).to_offset();
        block_device.write_bytes_at(inode_bitmap_offset, inner.inode_bitmap.as_bytes())?;

        // Write block bitmap
        let block_bitmap_offset = BlockId::new(inner.descriptor.block_bitmap).to_offset();
        block_device.write_bytes_at(block_bitmap_offset, inner.block_bitmap.as_bytes())?;

        inner.sync();
        Ok(())
    }

    pub fn sync_inodes(&self) -> Result<()> {
        let mut inode_cache = self.inode_cache.write();
        // Remove weak inodes
        inode_cache.retain(|_, inode| inode.upgrade().is_some());
        // Sync inodes
        for inode in inode_cache.values() {
            if let Some(inode) = inode.upgrade() {
                inode.sync_all()?;
            }
        }
        Ok(())
    }
}

impl Debug for BlockGroup {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("BlockGroup")
            .field("idx", &self.idx)
            .field("descriptor", &self.inner.read().descriptor)
            .field("block_bitmap", &self.inner.read().block_bitmap)
            .field("inode_bitmap", &self.inner.read().inode_bitmap)
            .finish()
    }
}

/// The Block Group Descriptor contains information regarding where important data
/// structures for that group are located.
///
/// The Block Group Descriptor Table contains a descriptor for each block group.
///
/// The table starts on the first block following the superblock.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct BlockGroupDescriptor {
    /// Blocks usage bitmap block
    pub block_bitmap: u32,
    /// Inodes usage bitmap block
    pub inode_bitmap: u32,
    /// Starting block of inode table
    pub inode_table: u32,
    /// Number of free blocks in group
    pub free_blocks_count: u16,
    /// Number of free inodes in group
    pub free_inodes_count: u16,
    /// Number of directories in group
    pub dirs_count: u16,
    pad: u16,
    reserved: [u32; 3],
}
