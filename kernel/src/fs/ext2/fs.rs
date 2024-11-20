// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use super::{
    block_group::{BlockGroup, RawGroupDescriptor},
    block_ptr::Ext2Bid,
    inode::{FilePerm, Inode, InodeDesc, RawInode},
    prelude::*,
    super_block::{RawSuperBlock, SuperBlock, SUPER_BLOCK_OFFSET},
};

/// The root inode number.
const ROOT_INO: u32 = 2;

/// The Ext2 filesystem.
#[derive(Debug)]
pub struct Ext2 {
    block_device: Arc<dyn BlockDevice>,
    super_block: RwMutex<Dirty<SuperBlock>>,
    block_groups: Vec<BlockGroup>,
    inodes_per_group: u32,
    blocks_per_group: Ext2Bid,
    inode_size: usize,
    block_size: usize,
    group_descriptors_segment: Segment,
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
        assert_eq!(
            super_block.block_size(),
            BLOCK_SIZE,
            "currently only support 4096-byte block size"
        );

        let group_descriptors_segment = {
            let npages = ((super_block.block_groups_count() as usize)
                * core::mem::size_of::<RawGroupDescriptor>())
            .div_ceil(BLOCK_SIZE);
            let segment = FrameAllocOptions::new(npages)
                .uninit(true)
                .alloc_contiguous()?;
            let bio_segment =
                BioSegment::new_from_segment(segment.clone(), BioDirection::FromDevice);
            match block_device.read_blocks(super_block.group_descriptors_bid(0), bio_segment)? {
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
                                 group_descriptors_segment: &Segment|
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
    pub fn blocks_per_group(&self) -> Ext2Bid {
        self.blocks_per_group
    }

    /// Returns the super block.
    pub fn super_block(&self) -> RwMutexReadGuard<Dirty<SuperBlock>> {
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
        inode_type: InodeType,
        file_perm: FilePerm,
    ) -> Result<Arc<Inode>> {
        let (block_group_idx, ino) =
            self.alloc_ino(dir_block_group_idx, inode_type == InodeType::Dir)?;
        let inode = {
            let inode_desc = InodeDesc::new(inode_type, file_perm);
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

    /// Allocates a consecutive range of blocks.
    ///
    /// The returned allocated range size may be smaller than the requested `count` if
    /// insufficient consecutive blocks are available.
    ///
    /// Attempts to allocate blocks from the `block_group_idx` group first.
    /// If allocation is not possible from this group, then search the remaining groups.
    pub(super) fn alloc_blocks(
        &self,
        mut block_group_idx: usize,
        count: Ext2Bid,
    ) -> Option<Range<Ext2Bid>> {
        if count > self.super_block.read().free_blocks_count() {
            return None;
        }

        let mut remaining_count = count;
        let mut allocated_range: Option<Range<Ext2Bid>> = None;
        for _ in 0..self.block_groups.len() {
            if remaining_count == 0 {
                break;
            }

            if block_group_idx >= self.block_groups.len() {
                block_group_idx = 0;
            }
            let block_group = &self.block_groups[block_group_idx];
            if let Some(range_in_group) = block_group.alloc_blocks(remaining_count) {
                let device_range = {
                    let start =
                        (block_group_idx as Ext2Bid) * self.blocks_per_group + range_in_group.start;
                    start..start + (range_in_group.len() as Ext2Bid)
                };
                match allocated_range {
                    Some(ref mut range) => {
                        if range.end == device_range.start {
                            // Accumulate consecutive bids
                            range.end = device_range.end;
                            remaining_count -= range_in_group.len() as Ext2Bid;
                        } else {
                            block_group.free_blocks(range_in_group);
                            break;
                        }
                    }
                    None => {
                        allocated_range = Some(device_range);
                    }
                }
            }
            block_group_idx += 1;
        }

        if let Some(range) = allocated_range.as_ref() {
            self.super_block
                .write()
                .dec_free_blocks(range.len() as Ext2Bid);
        }
        allocated_range
    }

    /// Frees a range of blocks.
    pub(super) fn free_blocks(&self, range: Range<Ext2Bid>) -> Result<()> {
        let mut current_range = range.clone();
        while !current_range.is_empty() {
            let (_, block_group) = self.block_group_of_bid(current_range.start)?;
            let range_in_group = {
                let start = self.block_idx(current_range.start);
                let len = (current_range.len() as Ext2Bid).min(self.blocks_per_group - start);
                start..start + len
            };
            // In order to prevent value underflow, it is necessary to increment
            // the free block counter prior to freeing the block.
            self.super_block
                .write()
                .inc_free_blocks(range_in_group.len() as Ext2Bid);
            block_group.free_blocks(range_in_group.clone());
            current_range.start += range_in_group.len() as Ext2Bid
        }

        Ok(())
    }

    /// Reads contiguous blocks starting from the `bid` synchronously.
    pub(super) fn read_blocks(&self, bid: Ext2Bid, bio_segment: BioSegment) -> Result<()> {
        let status = self
            .block_device
            .read_blocks(Bid::new(bid as u64), bio_segment)?;
        match status {
            BioStatus::Complete => Ok(()),
            err_status => Err(Error::from(err_status)),
        }
    }

    /// Reads contiguous blocks starting from the `bid` asynchronously.
    pub(super) fn read_blocks_async(
        &self,
        bid: Ext2Bid,
        bio_segment: BioSegment,
    ) -> Result<BioWaiter> {
        let waiter = self
            .block_device
            .read_blocks_async(Bid::new(bid as u64), bio_segment)?;
        Ok(waiter)
    }

    /// Writes contiguous blocks starting from the `bid` synchronously.
    pub(super) fn write_blocks(&self, bid: Ext2Bid, bio_segment: BioSegment) -> Result<()> {
        let status = self
            .block_device
            .write_blocks(Bid::new(bid as u64), bio_segment)?;
        match status {
            BioStatus::Complete => Ok(()),
            err_status => Err(Error::from(err_status)),
        }
    }

    /// Writes contiguous blocks starting from the `bid` asynchronously.
    pub(super) fn write_blocks_async(
        &self,
        bid: Ext2Bid,
        bio_segment: BioSegment,
    ) -> Result<BioWaiter> {
        let waiter = self
            .block_device
            .write_blocks_async(Bid::new(bid as u64), bio_segment)?;
        Ok(waiter)
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
            block_group.sync_metadata()?;
        }

        // Writes back the main superblock and group descriptor table.
        let mut bio_waiter = BioWaiter::new();
        let raw_super_block = RawSuperBlock::from((*super_block).deref());
        bio_waiter.concat(
            self.block_device
                .write_bytes_async(SUPER_BLOCK_OFFSET, raw_super_block.as_bytes())?,
        );
        let group_descriptors_bio_segment = BioSegment::new_from_segment(
            self.group_descriptors_segment.clone(),
            BioDirection::ToDevice,
        );
        bio_waiter.concat(self.block_device.write_blocks_async(
            super_block.group_descriptors_bid(0),
            group_descriptors_bio_segment.clone(),
        )?);
        bio_waiter
            .wait()
            .ok_or_else(|| Error::with_message(Errno::EIO, "failed to sync main metadata"))?;
        drop(bio_waiter);

        // Writes back the backups of superblock and group descriptor table.
        let mut raw_super_block_backup = raw_super_block;
        for idx in 1..super_block.block_groups_count() {
            if super_block.is_backup_group(idx as usize) {
                let mut bio_waiter = BioWaiter::new();
                raw_super_block_backup.block_group_idx = idx as u16;
                bio_waiter.concat(self.block_device.write_bytes_async(
                    super_block.bid(idx as usize).to_offset(),
                    raw_super_block_backup.as_bytes(),
                )?);
                bio_waiter.concat(self.block_device.write_blocks_async(
                    super_block.group_descriptors_bid(idx as usize),
                    group_descriptors_bio_segment.clone(),
                )?);
                bio_waiter.wait().ok_or_else(|| {
                    Error::with_message(Errno::EIO, "failed to sync backup metadata")
                })?;
            }
        }

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
    fn block_group_of_bid(&self, bid: Ext2Bid) -> Result<(usize, &BlockGroup)> {
        let block_group_idx = (bid / self.blocks_per_group) as usize;
        if block_group_idx >= self.block_groups.len() {
            return_errno_with_message!(Errno::EINVAL, "invalid bid");
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
    fn block_idx(&self, bid: Ext2Bid) -> Ext2Bid {
        bid % self.blocks_per_group
    }
}
