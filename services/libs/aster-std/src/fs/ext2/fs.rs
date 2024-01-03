// SPDX-License-Identifier: MPL-2.0

use super::block_group::{BlockGroup, RawGroupDescriptor};
use super::inode::{FilePerm, FileType, Inode, InodeDesc, RawInode};
use super::prelude::*;
use super::super_block::{RawSuperBlock, SuperBlock, SUPER_BLOCK_OFFSET};

/// The root inode number.
const ROOT_INO: u32 = 2;

/// The Ext2 filesystem.
#[derive(Debug)]
pub struct Ext2 {
    block_device: Arc<dyn BlockDevice>,
    super_block: RwMutex<Dirty<SuperBlock>>,
    block_groups: Vec<BlockGroup>,
    inodes_per_group: u32,
    blocks_per_group: u32,
    inode_size: usize,
    block_size: usize,
    group_descriptors_segment: VmSegment,
    self_ref: Weak<Self>,
}

impl Ext2 {
    /// Opens and loads an Ext2 from the `block_device`.
    pub fn open(block_device: Arc<dyn BlockDevice>) -> Result<Arc<Self>> {
        // Load the superblock
        // TODO: if the main superblock is corrupted, should we load the backup?
        let super_block = {
            let raw_super_block = block_device.read_val::<RawSuperBlock>(SUPER_BLOCK_OFFSET)?;
            SuperBlock::try_from(raw_super_block)?
        };
        assert!(super_block.block_size() == BLOCK_SIZE);

        let group_descriptors_segment = {
            let npages = ((super_block.block_groups_count() as usize)
                * core::mem::size_of::<RawGroupDescriptor>())
            .div_ceil(BLOCK_SIZE);
            let segment = VmAllocOptions::new(npages)
                .uninit(true)
                .is_contiguous(true)
                .alloc_contiguous()?;
            match block_device.read_blocks_sync(super_block.group_descriptors_bid(0), &segment)? {
                BioStatus::Complete => (),
                err_status => {
                    return Err(Error::from(err_status));
                }
            }
            segment
        };

        // Load the block groups information
        let load_block_groups = |fs: Weak<Ext2>,
                                 block_device: &dyn BlockDevice,
                                 group_descriptors_segment: &VmSegment|
         -> Result<Vec<BlockGroup>> {
            let block_groups_count = super_block.block_groups_count() as usize;
            let mut block_groups = Vec::with_capacity(block_groups_count);
            for idx in 0..block_groups_count {
                let block_group = BlockGroup::load(
                    group_descriptors_segment,
                    idx,
                    block_device,
                    &super_block,
                    fs.clone(),
                )?;
                block_groups.push(block_group);
            }
            Ok(block_groups)
        };

        let ext2 = Arc::new_cyclic(|weak_ref| Self {
            inodes_per_group: super_block.inodes_per_group(),
            blocks_per_group: super_block.blocks_per_group(),
            inode_size: super_block.inode_size(),
            block_size: super_block.block_size(),
            block_groups: load_block_groups(
                weak_ref.clone(),
                block_device.as_ref(),
                &group_descriptors_segment,
            )
            .unwrap(),
            block_device,
            super_block: RwMutex::new(Dirty::new(super_block)),
            group_descriptors_segment,
            self_ref: weak_ref.clone(),
        });
        Ok(ext2)
    }

    /// Returns the block device.
    pub fn block_device(&self) -> &dyn BlockDevice {
        self.block_device.as_ref()
    }

    /// Returns the size of block.
    pub fn block_size(&self) -> usize {
        self.block_size
    }

    /// Returns the size of inode.
    pub fn inode_size(&self) -> usize {
        self.inode_size
    }

    /// Returns the number of inodes in each block group.
    pub fn inodes_per_group(&self) -> u32 {
        self.inodes_per_group
    }

    /// Returns the number of blocks in each block group.
    pub fn blocks_per_group(&self) -> u32 {
        self.blocks_per_group
    }

    /// Returns the super block.
    pub fn super_block(&self) -> RwMutexReadGuard<'_, Dirty<SuperBlock>> {
        self.super_block.read()
    }

    /// Returns the root inode.
    pub fn root_inode(&self) -> Result<Arc<Inode>> {
        self.lookup_inode(ROOT_INO)
    }

    /// Finds and returns the inode by `ino`.
    pub(super) fn lookup_inode(&self, ino: u32) -> Result<Arc<Inode>> {
        let (_, block_group) = self.block_group_of_ino(ino)?;
        let inode_idx = self.inode_idx(ino);
        block_group.lookup_inode(inode_idx)
    }

    /// Creates a new inode.
    pub(super) fn create_inode(
        &self,
        dir_block_group_idx: usize,
        file_type: FileType,
        file_perm: FilePerm,
    ) -> Result<Arc<Inode>> {
        let (block_group_idx, ino) =
            self.alloc_ino(dir_block_group_idx, file_type == FileType::Dir)?;
        let inode = {
            let inode_desc = InodeDesc::new(file_type, file_perm);
            Inode::new(ino, block_group_idx, inode_desc, self.self_ref.clone())
        };
        let block_group = &self.block_groups[block_group_idx];
        block_group.insert_cache(self.inode_idx(ino), inode.clone());
        Ok(inode)
    }

    /// Allocates a new inode number, internally used by `new_inode`.
    ///
    /// Attempts to allocate from the `dir_block_group_idx` group first.
    /// If allocation is not possible from this group, then search the remaining groups.
    fn alloc_ino(&self, dir_block_group_idx: usize, is_dir: bool) -> Result<(usize, u32)> {
        let mut block_group_idx = dir_block_group_idx;
        if block_group_idx >= self.block_groups.len() {
            return_errno_with_message!(Errno::EINVAL, "invalid block group idx");
        }

        for _ in 0..self.block_groups.len() {
            if block_group_idx >= self.block_groups.len() {
                block_group_idx = 0;
            }
            let block_group = &self.block_groups[block_group_idx];
            if let Some(inode_idx) = block_group.alloc_inode(is_dir) {
                let ino = block_group_idx as u32 * self.inodes_per_group + inode_idx + 1;
                self.super_block.write().dec_free_inodes();
                return Ok((block_group_idx, ino));
            }
            block_group_idx += 1;
        }

        return_errno_with_message!(Errno::ENOSPC, "no space on device");
    }

    /// Frees an inode.
    pub(super) fn free_inode(&self, ino: u32, is_dir: bool) -> Result<()> {
        let (_, block_group) = self.block_group_of_ino(ino)?;
        let inode_idx = self.inode_idx(ino);
        // In order to prevent value underflow, it is necessary to increment
        // the free inode counter prior to freeing the inode.
        self.super_block.write().inc_free_inodes();
        block_group.free_inode(inode_idx, is_dir);
        Ok(())
    }

    /// Writes back the metadata of inode.
    pub(super) fn sync_inode(&self, ino: u32, inode: &InodeDesc) -> Result<()> {
        let (_, block_group) = self.block_group_of_ino(ino)?;
        let inode_idx = self.inode_idx(ino);
        block_group.sync_raw_inode(inode_idx, &RawInode::from(inode));
        Ok(())
    }

    /// Writes back the block group descriptor to the descriptors table.
    pub(super) fn sync_group_descriptor(
        &self,
        block_group_idx: usize,
        raw_descriptor: &RawGroupDescriptor,
    ) -> Result<()> {
        let offset = block_group_idx * core::mem::size_of::<RawGroupDescriptor>();
        self.group_descriptors_segment
            .write_val(offset, raw_descriptor)?;
        Ok(())
    }

    /// Allocates a new block.
    ///
    /// Attempts to allocate from the `block_group_idx` group first.
    /// If allocation is not possible from this group, then search the remaining groups.
    pub(super) fn alloc_block(&self, block_group_idx: usize) -> Result<Bid> {
        let mut block_group_idx = block_group_idx;
        if block_group_idx >= self.block_groups.len() {
            return_errno_with_message!(Errno::EINVAL, "invalid block group idx");
        }

        for _ in 0..self.block_groups.len() {
            if block_group_idx >= self.block_groups.len() {
                block_group_idx = 0;
            }
            let block_group = &self.block_groups[block_group_idx];
            if let Some(block_idx) = block_group.alloc_block() {
                let bid = block_group_idx as u32 * self.blocks_per_group + block_idx;
                self.super_block.write().dec_free_blocks();
                return Ok(Bid::new(bid as _));
            }
            block_group_idx += 1;
        }

        return_errno_with_message!(Errno::ENOSPC, "no space on device");
    }

    /// Frees a block.
    pub(super) fn free_block(&self, bid: Bid) -> Result<()> {
        let (_, block_group) = self.block_group_of_bid(bid)?;
        let block_idx = self.block_idx(bid);
        // In order to prevent value underflow, it is necessary to increment
        // the free block counter prior to freeing the block.
        self.super_block.write().inc_free_blocks();
        block_group.free_block(block_idx);
        Ok(())
    }

    /// Reads contiguous blocks starting from the `bid` synchronously.
    pub(super) fn read_blocks(&self, bid: Bid, segment: &VmSegment) -> Result<()> {
        let status = self.block_device.read_blocks_sync(bid, segment)?;
        match status {
            BioStatus::Complete => Ok(()),
            err_status => Err(Error::from(err_status)),
        }
    }

    /// Reads one block indicated by the `bid` synchronously.
    pub(super) fn read_block(&self, bid: Bid, frame: &VmFrame) -> Result<()> {
        let status = self.block_device.read_block_sync(bid, frame)?;
        match status {
            BioStatus::Complete => Ok(()),
            err_status => Err(Error::from(err_status)),
        }
    }

    /// Writes contiguous blocks starting from the `bid` synchronously.
    pub(super) fn write_blocks(&self, bid: Bid, segment: &VmSegment) -> Result<()> {
        let status = self.block_device.write_blocks_sync(bid, segment)?;
        match status {
            BioStatus::Complete => Ok(()),
            err_status => Err(Error::from(err_status)),
        }
    }

    /// Writes one block indicated by the `bid` synchronously.
    pub(super) fn write_block(&self, bid: Bid, frame: &VmFrame) -> Result<()> {
        let status = self.block_device.write_block_sync(bid, frame)?;
        match status {
            BioStatus::Complete => Ok(()),
            err_status => Err(Error::from(err_status)),
        }
    }

    /// Writes back the metadata to the block device.
    pub fn sync_metadata(&self) -> Result<()> {
        // If the superblock is clean, the block groups must be clean.
        if !self.super_block.read().is_dirty() {
            return Ok(());
        }

        let mut super_block = self.super_block.write();
        // Writes back the metadata of block groups
        for block_group in &self.block_groups {
            block_group.sync_metadata(&super_block)?;
        }

        let mut bio_waiter = BioWaiter::new();
        // Writes back the main superblock and group descriptor table.
        let raw_super_block = RawSuperBlock::from((*super_block).deref());
        bio_waiter.concat(
            self.block_device
                .write_bytes_async(SUPER_BLOCK_OFFSET, raw_super_block.as_bytes())?,
        );
        bio_waiter.concat(self.block_device.write_blocks(
            super_block.group_descriptors_bid(0),
            &self.group_descriptors_segment,
        )?);

        // Writes back the backups of superblock and group descriptor table.
        let mut raw_super_block_backup = raw_super_block;
        for idx in 1..super_block.block_groups_count() {
            if super_block.is_backup_group(idx as usize) {
                raw_super_block_backup.block_group_idx = idx as u16;
                bio_waiter.concat(self.block_device.write_bytes_async(
                    super_block.bid(idx as usize).to_offset(),
                    raw_super_block_backup.as_bytes(),
                )?);
                bio_waiter.concat(self.block_device.write_blocks(
                    super_block.group_descriptors_bid(idx as usize),
                    &self.group_descriptors_segment,
                )?);
            }
        }

        // Waits for the completion of all submitted bios.
        bio_waiter
            .wait()
            .ok_or_else(|| Error::with_message(Errno::EIO, "failed to sync metadata of fs"))?;

        // Reset to clean.
        super_block.clear_dirty();
        Ok(())
    }

    /// Writes back all the cached inodes to the block device.
    pub fn sync_all_inodes(&self) -> Result<()> {
        for block_group in &self.block_groups {
            block_group.sync_all_inodes()?;
        }
        Ok(())
    }

    #[inline]
    fn block_group_of_bid(&self, bid: Bid) -> Result<(usize, &BlockGroup)> {
        let block_group_idx = (bid.to_raw() / (self.blocks_per_group as u64)) as usize;
        if block_group_idx >= self.block_groups.len() {
            return_errno!(Errno::ENOENT);
        }
        Ok((block_group_idx, &self.block_groups[block_group_idx]))
    }

    #[inline]
    fn block_group_of_ino(&self, ino: u32) -> Result<(usize, &BlockGroup)> {
        let block_group_idx = ((ino - 1) / self.inodes_per_group) as usize;
        if block_group_idx >= self.block_groups.len() {
            return_errno!(Errno::ENOENT);
        }
        Ok((block_group_idx, &self.block_groups[block_group_idx]))
    }

    #[inline]
    fn inode_idx(&self, ino: u32) -> u32 {
        (ino - 1) % self.inodes_per_group
    }

    #[inline]
    fn block_idx(&self, bid: Bid) -> u32 {
        (bid.to_raw() as u32) % self.blocks_per_group
    }
}
