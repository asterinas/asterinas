// SPDX-License-Identifier: MPL-2.0

//! Core Ext4 filesystem object.

use device_id::DeviceId;

use super::{
    block_group::{BlockGroup, RawGroupDescriptor},
    inode::Inode as Ext4Inode,
    prelude::*,
    super_block::{RawSuperBlock, SUPER_BLOCK_OFFSET, SuperBlock},
};
use crate::fs::vfs::{
    file_system::{FileSystem, FsEventSubscriberStats},
    registry::{FsCreationCtx, FsProperties, FsType},
};

/// The root inode number.
const ROOT_INO: u32 = 2;

/// The Ext4 filesystem.
#[derive(Debug)]
pub struct Ext4 {
    block_device: Arc<dyn BlockDevice>,
    super_block: RwMutex<Dirty<SuperBlock>>,
    block_groups: Vec<BlockGroup>,
    inodes_per_group: u32,
    blocks_per_group: u64,
    inode_size: usize,
    block_size: usize,
    group_descriptors_segment: USegment,
    fs_event_subscriber_stats: FsEventSubscriberStats,
    self_ref: Weak<Self>,
}

impl Ext4 {
    /// Opens and loads an Ext4 filesystem from the `block_device`.
    pub fn open(block_device: Arc<dyn BlockDevice>) -> Result<Arc<Self>> {
        let super_block = {
            let raw_super_block = block_device.read_val::<RawSuperBlock>(SUPER_BLOCK_OFFSET)?;
            SuperBlock::try_from(raw_super_block)?
        };

        let block_size = super_block.block_size();
        let group_descriptors_segment: USegment = {
            let desc_size = super_block.desc_size();
            let npages = ((super_block.block_groups_count() as usize) * desc_size)
                .div_ceil(block_size);
            let segment = FrameAllocOptions::new()
                .zeroed(false)
                .alloc_segment(npages)?;
            let bio_segment =
                BioSegment::new_from_segment(segment.clone().into(), BioDirection::FromDevice);
            let gdt_bid = super_block.group_descriptors_bid(0);
            match block_device.read_blocks(Bid::new(gdt_bid), bio_segment)? {
                BioStatus::Complete => (),
                err_status => {
                    return Err(Error::from(err_status));
                }
            }
            segment.into()
        };

        let load_block_groups = |fs: Weak<Ext4>,
                                 block_device: &dyn BlockDevice,
                                 group_descriptors_segment: &USegment|
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

        let ext4 = Arc::new_cyclic(|weak_ref| Self {
            inodes_per_group: super_block.inodes_per_group(),
            blocks_per_group: super_block.blocks_per_group() as u64,
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
            fs_event_subscriber_stats: FsEventSubscriberStats::new(),
            self_ref: weak_ref.clone(),
        });
        Ok(ext4)
    }

    /// Returns the block device.
    pub fn block_device(&self) -> &dyn BlockDevice {
        self.block_device.as_ref()
    }

    /// Returns the device ID containing this filesystem.
    pub fn container_device_id(&self) -> DeviceId {
        self.block_device.id()
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
    pub fn blocks_per_group(&self) -> u64 {
        self.blocks_per_group
    }

    /// Returns the super block.
    pub fn super_block(&self) -> RwMutexReadGuard<'_, Dirty<SuperBlock>> {
        self.super_block.read()
    }

    /// Returns the root inode.
    pub fn root_inode(&self) -> Result<Arc<Ext4Inode>> {
        self.lookup_inode(ROOT_INO)
    }

    /// Finds and returns the inode by `ino`.
    pub(super) fn lookup_inode(&self, ino: u32) -> Result<Arc<Ext4Inode>> {
        let (_, block_group) = self.block_group_of_ino(ino)?;
        let inode_idx = self.inode_idx(ino);
        block_group.lookup_inode(inode_idx)
    }

    /// Reads contiguous blocks starting from the `bid` synchronously.
    pub(super) fn read_blocks(&self, bid: u64, bio_segment: BioSegment) -> Result<()> {
        let status = self
            .block_device
            .read_blocks(Bid::new(bid), bio_segment)?;
        match status {
            BioStatus::Complete => Ok(()),
            err_status => Err(Error::from(err_status)),
        }
    }

    /// Reads contiguous blocks starting from the `bid` asynchronously.
    pub(super) fn read_blocks_async(
        &self,
        bid: u64,
        bio_segment: BioSegment,
    ) -> Result<BioWaiter> {
        let waiter = self
            .block_device
            .read_blocks_async(Bid::new(bid), bio_segment)?;
        Ok(waiter)
    }

    fn block_group_of_ino(&self, ino: u32) -> Result<(usize, &BlockGroup)> {
        let block_group_idx = ((ino - 1) / self.inodes_per_group) as usize;
        if block_group_idx >= self.block_groups.len() {
            return_errno!(Errno::ENOENT);
        }
        Ok((block_group_idx, &self.block_groups[block_group_idx]))
    }

    fn inode_idx(&self, ino: u32) -> u32 {
        (ino - 1) % self.inodes_per_group
    }

    pub fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        &self.fs_event_subscriber_stats
    }
}

pub(super) struct Ext4Type;

impl FsType for Ext4Type {
    fn name(&self) -> &'static str {
        "ext4"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::NEED_DISK
    }

    fn create(&self, fs_creation_ctx: &FsCreationCtx) -> Result<Arc<dyn FileSystem>> {
        Ok(Ext4::open(fs_creation_ctx.resolve_block_device()?)?)
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        None
    }
}
