use crate::block_group::BlockGroup;
use crate::inode::{Ext2Inode, FilePerm, FileType, RawInode};
use crate::prelude::*;
use crate::super_block::{Ext2SuperBlock, SUPER_BLOCK_OFFSET};

pub const EXT2_ROOT_INO: u32 = 2;

/// The Ext2 filesystem
#[derive(Debug)]
pub struct Ext2 {
    block_device: Box<dyn BlockDevice>,
    super_block: RwLock<Dirty<Ext2SuperBlock>>,
    block_groups: Vec<BlockGroup>,
    self_ref: Weak<Self>,
}

impl Ext2 {
    /// Load an Ext2 from a block device.
    pub fn open(block_device: Box<dyn BlockDevice>) -> Result<Arc<Self>> {
        let super_block = block_device.read_val::<Ext2SuperBlock>(SUPER_BLOCK_OFFSET)?;
        super_block.validate()?;

        let block_groups = {
            debug_assert!(super_block.block_size() == BLOCK_SIZE);

            let block_groups_count = super_block.block_groups_count() as usize;
            let mut block_groups = Vec::with_capacity(block_groups_count);
            for idx in 0..block_groups_count {
                let block_group = BlockGroup::new(idx, block_device.as_ref(), &super_block)?;
                block_groups.push(block_group);
            }
            block_groups
        };

        let ext2 = Arc::new_cyclic(|weak_ref| Self {
            block_device,
            super_block: RwLock::new(Dirty::new(super_block)),
            block_groups,
            self_ref: weak_ref.clone(),
        });
        Ok(ext2)
    }

    pub fn block_device(&self) -> &dyn BlockDevice {
        self.block_device.as_ref()
    }

    pub fn super_block(&self) -> Ext2SuperBlock {
        **self.super_block.read()
    }

    pub fn root_inode<P: PageCache>(&self) -> Result<Arc<Ext2Inode>> {
        self.find_inode::<P>(EXT2_ROOT_INO)
    }

    pub(crate) fn find_inode<P: PageCache>(&self, ino: u32) -> Result<Arc<Ext2Inode>> {
        let block_group = self.block_group_of_inode(ino)?;
        let inode_idx = self.inode_idx(ino);
        let inode_option = block_group.get_inode(inode_idx)?;
        if let Some(inode) = inode_option {
            return Ok(inode);
        }

        // Load it from block device.
        let inode = {
            let inode_offset = self.inode_offset(block_group, inode_idx);
            let raw_inode = self.block_device.read_val::<RawInode>(inode_offset)?;
            Ext2Inode::new::<P>(ino, Dirty::new(raw_inode), self.self_ref.upgrade().unwrap())
        };
        block_group.put_inode(inode_idx, Arc::downgrade(&inode));
        Ok(inode)
    }

    pub(crate) fn new_inode<P: PageCache>(
        &self,
        dir_block_group_idx: u32,
        file_type: FileType,
        file_perm: FilePerm,
    ) -> Result<Arc<Ext2Inode>> {
        let (block_group, ino) = self.alloc_ino(dir_block_group_idx, file_type == FileType::Dir)?;
        let inode = {
            let raw_inode = RawInode::new(file_type, file_perm);
            Ext2Inode::new::<P>(
                ino,
                Dirty::new_dirty(raw_inode),
                self.self_ref.upgrade().unwrap(),
            )
        };
        block_group.put_inode(self.inode_idx(ino), Arc::downgrade(&inode));
        Ok(inode)
    }

    pub(crate) fn free_inode(&self, ino: u32, is_dir: bool) -> Result<()> {
        let block_group = self.block_group_of_inode(ino)?;
        let inode_idx = self.inode_idx(ino);
        block_group.free_inode(inode_idx, is_dir);
        self.super_block.write().inc_free_inodes();
        Ok(())
    }

    pub(crate) fn flush_raw_inode(&self, ino: u32, raw_inode: &RawInode) -> Result<()> {
        let block_group = self.block_group_of_inode(ino)?;
        let inode_idx = self.inode_idx(ino);
        let inode_offset = self.inode_offset(block_group, inode_idx);
        self.block_device.write_val(inode_offset, raw_inode)?;
        Ok(())
    }

    fn alloc_ino(&self, dir_block_group_idx: u32, is_dir: bool) -> Result<(&BlockGroup, u32)> {
        let mut block_group_idx = dir_block_group_idx as usize;
        loop {
            if block_group_idx >= self.block_groups.len() {
                return Err(Error::NoSpace);
            }
            let block_group = &self.block_groups[block_group_idx];
            if let Some(inode_idx) = block_group.alloc_inode(is_dir) {
                let ino = block_group_idx as u32 * self.super_block.read().inodes_per_group
                    + inode_idx
                    + 1;
                self.super_block.write().dec_free_inodes();
                return Ok((block_group, ino));
            }
            block_group_idx += 1;
        }
    }

    pub(crate) fn alloc_block(&self, block_group_idx: u32) -> Result<BlockId> {
        let mut block_group_idx = block_group_idx as usize;
        loop {
            if block_group_idx >= self.block_groups.len() {
                return Err(Error::NoSpace);
            }
            let block_group = &self.block_groups[block_group_idx];
            if let Some(block_idx) = block_group.alloc_block() {
                let bid =
                    block_group_idx as u32 * self.super_block.read().blocks_per_group + block_idx;
                self.super_block.write().dec_free_blocks();
                return Ok(BlockId::new(bid));
            }
            block_group_idx += 1;
        }
    }

    pub(crate) fn free_block(&self, bid: BlockId) -> Result<()> {
        let block_group = self.block_group(bid)?;
        let block_idx = self.block_idx(bid);
        block_group.free_block(block_idx);
        self.super_block.write().inc_free_blocks();
        Ok(())
    }

    pub fn sync_metadata(&self) -> Result<()> {
        // Write SuperBlock
        if self.super_block.read().is_dirty() {
            let mut super_block = self.super_block.write();
            self.block_device
                .write_val(SUPER_BLOCK_OFFSET, (*super_block).deref())?;
            // Write SuperBlock backups
            let mut super_block_backup = **super_block;
            for idx in 1..super_block.block_groups_count() {
                if super_block.is_backup_block_group(idx) {
                    super_block_backup.block_group = idx as u16;
                    let offset = (idx * super_block.blocks_per_group) as usize;
                    self.block_device.write_val(offset, &super_block_backup)?;
                }
            }
            super_block.sync();
        }

        // Write BlockGroup
        for block_group in &self.block_groups {
            block_group.sync_metadata(self.block_device.as_ref(), &self.super_block.read())?;
        }
        Ok(())
    }

    pub fn sync_inodes(&self) -> Result<()> {
        for block_group in &self.block_groups {
            block_group.sync_inodes()?;
        }
        Ok(())
    }

    fn block_group(&self, bid: BlockId) -> Result<&BlockGroup> {
        let block_group_idx = self.block_group_idx(bid) as usize;
        if block_group_idx >= self.block_groups.len() {
            return Err(Error::NotFound);
        }
        Ok(&self.block_groups[block_group_idx])
    }

    fn block_group_of_inode(&self, ino: u32) -> Result<&BlockGroup> {
        let block_group_idx = self.block_group_idx_of_ino(ino) as usize;
        if block_group_idx >= self.block_groups.len() {
            return Err(Error::NotFound);
        }
        Ok(&self.block_groups[block_group_idx])
    }

    #[inline]
    pub(crate) fn block_group_idx_of_ino(&self, ino: u32) -> u32 {
        (ino - 1) / self.super_block.read().inodes_per_group
    }

    #[inline]
    fn inode_offset(&self, block_group: &BlockGroup, inode_idx: u32) -> usize {
        block_group.inode_table_bid().to_offset()
            + self.super_block.read().inode_size as usize * inode_idx as usize
    }

    #[inline]
    fn inode_idx(&self, ino: u32) -> u32 {
        (ino - 1) % self.super_block.read().inodes_per_group
    }

    #[inline]
    fn block_group_idx(&self, bid: BlockId) -> u32 {
        let bid: u32 = bid.into();
        bid / self.super_block.read().blocks_per_group
    }

    #[inline]
    fn block_idx(&self, bid: BlockId) -> u32 {
        let bid: u32 = bid.into();
        bid % self.super_block.read().blocks_per_group
    }
}

impl Drop for Ext2 {
    fn drop(&mut self) {
        self.sync_metadata().unwrap();
    }
}
