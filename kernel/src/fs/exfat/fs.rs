// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]
#![allow(unused_variables)]

use core::{num::NonZeroUsize, ops::Range, sync::atomic::AtomicU64};

use aster_block::{
    bio::{BioDirection, BioSegment, BioWaiter},
    id::BlockId,
    BlockDevice,
};
use hashbrown::HashMap;
use lru::LruCache;
use ostd::mm::Frame;
pub(super) use ostd::mm::VmIo;

use super::{
    bitmap::ExfatBitmap,
    fat::{ClusterID, ExfatChain, FatChainFlags, FatValue, FAT_ENTRY_SIZE},
    inode::ExfatInode,
    super_block::{ExfatBootSector, ExfatSuperBlock},
    upcase_table::ExfatUpcaseTable,
};
use crate::{
    fs::{
        exfat::{constants::*, inode::Ino},
        utils::{FileSystem, FsFlags, Inode, PageCache, PageCacheBackend, SuperBlock},
    },
    prelude::*,
};

#[derive(Debug)]
pub struct ExfatFS {
    block_device: Arc<dyn BlockDevice>,
    super_block: ExfatSuperBlock,

    bitmap: Arc<Mutex<ExfatBitmap>>,

    upcase_table: Arc<SpinLock<ExfatUpcaseTable>>,

    mount_option: ExfatMountOptions,
    //Used for inode allocation.
    highest_inode_number: AtomicU64,

    //inodes are indexed by their hash_value.
    inodes: RwMutex<HashMap<usize, Arc<ExfatInode>>>,

    //Cache for fat table
    fat_cache: RwLock<LruCache<ClusterID, ClusterID>>,
    meta_cache: PageCache,

    //A global lock, We need to hold the mutex before accessing bitmap or inode, otherwise there will be deadlocks.
    mutex: Mutex<()>,
}

const FAT_LRU_CACHE_SIZE: usize = 1024;

pub(super) const EXFAT_ROOT_INO: Ino = 1;

impl ExfatFS {
    pub fn open(
        block_device: Arc<dyn BlockDevice>,
        mount_option: ExfatMountOptions,
    ) -> Result<Arc<Self>> {
        // Load the super_block
        let super_block = Self::read_super_block(block_device.as_ref())?;
        let fs_size = super_block.num_clusters as usize * super_block.cluster_size as usize;
        let exfat_fs = Arc::new_cyclic(|weak_self| ExfatFS {
            block_device,
            super_block,
            bitmap: Arc::new(Mutex::new(ExfatBitmap::default())),
            upcase_table: Arc::new(SpinLock::new(ExfatUpcaseTable::empty())),
            mount_option,
            highest_inode_number: AtomicU64::new(EXFAT_ROOT_INO + 1),
            inodes: RwMutex::new(HashMap::new()),
            fat_cache: RwLock::new(LruCache::<ClusterID, ClusterID>::new(
                NonZeroUsize::new(FAT_LRU_CACHE_SIZE).unwrap(),
            )),
            meta_cache: PageCache::with_capacity(fs_size, weak_self.clone() as _).unwrap(),
            mutex: Mutex::new(()),
        });

        // TODO: if the main superblock is corrupted, should we load the backup?

        // Verify boot region
        Self::verify_boot_region(exfat_fs.block_device())?;

        let weak_fs = Arc::downgrade(&exfat_fs);

        let root_chain = ExfatChain::new(
            weak_fs.clone(),
            super_block.root_dir,
            None,
            FatChainFlags::ALLOC_POSSIBLE,
        )?;

        let root = ExfatInode::build_root_inode(weak_fs.clone(), root_chain.clone())?;

        let upcase_table = ExfatUpcaseTable::load(
            weak_fs.clone(),
            root.page_cache().unwrap(),
            root_chain.clone(),
        )?;

        let bitmap = ExfatBitmap::load(
            weak_fs.clone(),
            root.page_cache().unwrap(),
            root_chain.clone(),
        )?;

        *exfat_fs.bitmap.lock() = bitmap;
        *exfat_fs.upcase_table.lock() = upcase_table;

        // TODO: Handle UTF-8

        // TODO: Init NLS Table

        exfat_fs.inodes.write().insert(root.hash_index(), root);

        Ok(exfat_fs)
    }

    pub(super) fn alloc_inode_number(&self) -> Ino {
        self.highest_inode_number
            .fetch_add(1, core::sync::atomic::Ordering::SeqCst)
    }

    pub(super) fn find_opened_inode(&self, hash: usize) -> Option<Arc<ExfatInode>> {
        self.inodes.read().get(&hash).cloned()
    }

    pub(super) fn remove_inode(&self, hash: usize) {
        let _ = self.inodes.write().remove(&hash);
    }

    pub(super) fn evict_inode(&self, hash: usize) -> Result<()> {
        if let Some(inode) = self.inodes.read().get(&hash).cloned() {
            if inode.is_deleted() {
                inode.reclaim_space()?;
            } else {
                inode.sync_all()?;
            }
        }
        self.inodes.write().remove(&hash);
        Ok(())
    }

    pub(super) fn insert_inode(&self, inode: Arc<ExfatInode>) -> Option<Arc<ExfatInode>> {
        self.inodes.write().insert(inode.hash_index(), inode)
    }

    pub(super) fn sync_meta_at(&self, range: core::ops::Range<usize>) -> Result<()> {
        self.meta_cache.pages().decommit(range)?;
        Ok(())
    }

    pub(super) fn write_meta_at(&self, offset: usize, buf: &[u8]) -> Result<()> {
        self.meta_cache.pages().write_bytes(offset, buf)?;
        Ok(())
    }

    pub(super) fn read_meta_at(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        self.meta_cache.pages().read_bytes(offset, buf)?;
        Ok(())
    }

    pub(super) fn read_next_fat(&self, cluster: ClusterID) -> Result<FatValue> {
        {
            let mut cache_inner = self.fat_cache.write();

            let cache = cache_inner.get(&cluster);
            if let Some(&value) = cache {
                return Ok(FatValue::from(value));
            }
        }

        let sb: ExfatSuperBlock = self.super_block();
        let sector_size = sb.sector_size;

        if !self.is_valid_cluster(cluster) {
            return_errno_with_message!(Errno::EIO, "invalid access to FAT")
        }

        let position =
            sb.fat1_start_sector * sector_size as u64 + (cluster as u64) * FAT_ENTRY_SIZE as u64;
        let mut buf: [u8; FAT_ENTRY_SIZE] = [0; FAT_ENTRY_SIZE];
        self.read_meta_at(position as usize, &mut buf)?;

        let value = u32::from_le_bytes(buf);
        self.fat_cache.write().put(cluster, value);

        Ok(FatValue::from(value))
    }

    pub(super) fn write_next_fat(
        &self,
        cluster: ClusterID,
        value: FatValue,
        sync: bool,
    ) -> Result<()> {
        let sb: ExfatSuperBlock = self.super_block();
        let sector_size = sb.sector_size;
        let raw_value: u32 = value.into();

        // We expect the fat table to change less frequently, so we write its content to disk immediately instead of absorbing it.
        let position =
            sb.fat1_start_sector * sector_size as u64 + (cluster as u64) * FAT_ENTRY_SIZE as u64;

        self.write_meta_at(position as usize, &raw_value.to_le_bytes())?;
        if sync {
            self.sync_meta_at(position as usize..position as usize + FAT_ENTRY_SIZE)?;
        }

        if sb.fat1_start_sector != sb.fat2_start_sector {
            let mirror_position = sb.fat2_start_sector * sector_size as u64
                + (cluster as u64) * FAT_ENTRY_SIZE as u64;
            self.write_meta_at(mirror_position as usize, &raw_value.to_le_bytes())?;
            if sync {
                self.sync_meta_at(
                    mirror_position as usize..mirror_position as usize + FAT_ENTRY_SIZE,
                )?;
            }
        }

        self.fat_cache.write().put(cluster, raw_value);

        Ok(())
    }

    fn verify_boot_region(block_device: &dyn BlockDevice) -> Result<()> {
        // TODO: Check boot signature and boot checksum.
        Ok(())
    }

    fn read_super_block(block_device: &dyn BlockDevice) -> Result<ExfatSuperBlock> {
        let boot_sector = block_device.read_val::<ExfatBootSector>(0)?;
        /* Check the validity of BOOT */
        if boot_sector.signature != BOOT_SIGNATURE {
            return_errno_with_message!(Errno::EINVAL, "invalid boot record signature");
        }

        if !boot_sector.fs_name.eq(STR_EXFAT.as_bytes()) {
            return_errno_with_message!(Errno::EINVAL, "invalid fs name");
        }

        /*
         * must_be_zero field must be filled with zero to prevent mounting
         * from FAT volume.
         */
        if boot_sector.must_be_zero.iter().any(|&x| x != 0) {
            return_errno_with_message!(
                Errno::EINVAL,
                "must_be_zero field must be filled with zero"
            );
        }

        if boot_sector.num_fats != 1 && boot_sector.num_fats != 2 {
            return_errno_with_message!(Errno::EINVAL, "bogus number of FAT structure");
        }

        // sect_size_bits could be at least 9 and at most 12.
        if boot_sector.sector_size_bits < EXFAT_MIN_SECT_SIZE_BITS
            || boot_sector.sector_size_bits > EXFAT_MAX_SECT_SIZE_BITS
        {
            return_errno_with_message!(Errno::EINVAL, "bogus sector size bits");
        }

        if boot_sector.sector_per_cluster_bits + boot_sector.sector_size_bits > 25 {
            return_errno_with_message!(Errno::EINVAL, "bogus sector size bits per cluster");
        }

        let super_block = ExfatSuperBlock::try_from(boot_sector)?;

        /* Check consistencies */
        if ((super_block.num_fat_sectors as u64) << boot_sector.sector_size_bits)
            < (super_block.num_clusters as u64) * 4
        {
            return_errno_with_message!(Errno::EINVAL, "bogus fat length");
        }

        if super_block.data_start_sector
            < super_block.fat1_start_sector
                + (super_block.num_fat_sectors as u64 * boot_sector.num_fats as u64)
        {
            return_errno_with_message!(Errno::EINVAL, "bogus data start vector");
        }

        if (super_block.vol_flags & VOLUME_DIRTY as u32) != 0 {
            warn!("Volume was not properly unmounted. Some data may be corrupt. Please run fsck.")
        }

        if (super_block.vol_flags & MEDIA_FAILURE as u32) != 0 {
            warn!("Medium has reported failures. Some data may be lost.")
        }

        Self::calibrate_blocksize(&super_block, 1 << boot_sector.sector_size_bits)?;

        Ok(super_block)
    }

    fn calibrate_blocksize(super_block: &ExfatSuperBlock, logical_sec: u32) -> Result<()> {
        // TODO: logical_sect should be larger than block_size.
        Ok(())
    }

    pub(super) fn block_device(&self) -> &dyn BlockDevice {
        self.block_device.as_ref()
    }

    pub(super) fn super_block(&self) -> ExfatSuperBlock {
        self.super_block
    }

    pub(super) fn bitmap(&self) -> Arc<Mutex<ExfatBitmap>> {
        self.bitmap.clone()
    }

    pub(super) fn upcase_table(&self) -> Arc<SpinLock<ExfatUpcaseTable>> {
        self.upcase_table.clone()
    }

    pub(super) fn root_inode(&self) -> Arc<ExfatInode> {
        self.inodes.read().get(&ROOT_INODE_HASH).unwrap().clone()
    }

    pub(super) fn sector_size(&self) -> usize {
        self.super_block.sector_size as usize
    }

    pub(super) fn fs_size(&self) -> usize {
        self.super_block.cluster_size as usize * self.super_block.num_clusters as usize
    }

    pub(super) fn lock(&self) -> MutexGuard<()> {
        self.mutex.lock()
    }

    pub(super) fn cluster_size(&self) -> usize {
        self.super_block.cluster_size as usize
    }

    pub(super) fn num_free_clusters(&self) -> u32 {
        self.bitmap.lock().num_free_clusters()
    }

    pub(super) fn cluster_to_off(&self, cluster: u32) -> usize {
        (((((cluster - EXFAT_RESERVED_CLUSTERS) as u64) << self.super_block.sect_per_cluster_bits)
            + self.super_block.data_start_sector)
            * self.super_block.sector_size as u64) as usize
    }

    pub(super) fn is_valid_cluster(&self, cluster: u32) -> bool {
        cluster >= EXFAT_RESERVED_CLUSTERS && cluster <= self.super_block.num_clusters
    }

    pub(super) fn is_cluster_range_valid(&self, clusters: Range<ClusterID>) -> bool {
        clusters.start >= EXFAT_RESERVED_CLUSTERS && clusters.end <= self.super_block.num_clusters
    }

    pub(super) fn set_volume_dirty(&mut self) {
        todo!();
    }

    pub fn mount_option(&self) -> ExfatMountOptions {
        self.mount_option.clone()
    }
}

impl PageCacheBackend for ExfatFS {
    fn read_page_async(&self, idx: usize, frame: &Frame) -> Result<BioWaiter> {
        if self.fs_size() < idx * PAGE_SIZE {
            return_errno_with_message!(Errno::EINVAL, "invalid read size")
        }
        let bio_segment =
            BioSegment::new_from_segment(frame.clone().into(), BioDirection::FromDevice);
        let waiter = self
            .block_device
            .read_blocks_async(BlockId::new(idx as u64), bio_segment)?;
        Ok(waiter)
    }

    fn write_page_async(&self, idx: usize, frame: &Frame) -> Result<BioWaiter> {
        if self.fs_size() < idx * PAGE_SIZE {
            return_errno_with_message!(Errno::EINVAL, "invalid write size")
        }
        let bio_segment =
            BioSegment::new_from_segment(frame.clone().into(), BioDirection::ToDevice);
        let waiter = self
            .block_device
            .write_blocks_async(BlockId::new(idx as u64), bio_segment)?;
        Ok(waiter)
    }

    fn npages(&self) -> usize {
        self.fs_size() / PAGE_SIZE
    }
}

impl FileSystem for ExfatFS {
    fn sync(&self) -> Result<()> {
        for inode in self.inodes.read().values() {
            inode.sync_all()?;
        }
        self.meta_cache.evict_range(0..self.fs_size())?;
        Ok(())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root_inode()
    }

    fn sb(&self) -> SuperBlock {
        SuperBlock::new(BOOT_SIGNATURE as u64, self.sector_size(), MAX_NAME_LENGTH)
    }

    fn flags(&self) -> FsFlags {
        FsFlags::DENTRY_UNEVICTABLE
    }
}

#[derive(Clone, Debug, Default)]
// Error handling
pub enum ExfatErrorMode {
    #[default]
    Continue,
    Panic,
    ReadOnly,
}

#[derive(Clone, Debug, Default)]
//Mount options
pub struct ExfatMountOptions {
    pub(super) fs_uid: usize,
    pub(super) fs_gid: usize,
    pub(super) fs_fmask: u16,
    pub(super) fs_dmask: u16,
    pub(super) allow_utime: u16,
    pub(super) iocharset: String,
    pub(super) errors: ExfatErrorMode,
    pub(super) utf8: bool,
    pub(super) sys_tz: bool,
    pub(super) discard: bool,
    pub(super) keep_last_dots: bool,
    pub(super) time_offset: i32,
    pub(super) zero_size_dir: bool,
}
