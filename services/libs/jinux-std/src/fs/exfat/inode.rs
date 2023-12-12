use crate::fs::exfat::dentry::ExfatDentryIterator;
use crate::fs::exfat::fat::ExfatChain;

use crate::fs::exfat::fs::ExfatFS;
use crate::fs::utils::{Inode, InodeType, Metadata, PageCache};
use crate::time::now_as_duration;

use super::bitmap::ExfatBitmap;
use super::block_device::{is_block_aligned, SECTOR_SIZE};
use super::constants::*;
use super::dentry::{Checksum, ExfatDentry, ExfatDentrySet, ExfatName, DENTRY_SIZE};
use super::fat::{ClusterID, ExfatChainPosition, FatChainFlags, FatTrait, FatValue};
use super::fs::{ExfatMountOptions, EXFAT_ROOT_INO};
use super::utils::{make_hash_index, DosTimestamp};
use crate::events::IoEvents;
use crate::fs::device::Device;
use crate::fs::utils::DirentVisitor;
use crate::fs::utils::InodeMode;
use crate::fs::utils::IoctlCmd;
use crate::prelude::*;
use crate::process::signal::Poller;
use crate::vm::vmo::Vmo;
pub(super) use align_ext::AlignExt;
use alloc::string::String;
use core::cmp::Ordering;
use core::time::Duration;
use jinux_frame::vm::{VmAllocOptions, VmFrame, VmIo};
use jinux_rights::Full;

///Inode number
pub type Ino = usize;

bitflags! {
    pub struct FatAttr : u16{
        const READONLY = 0x0001;
        const HIDDEN   = 0x0002;
        const SYSTEM   = 0x0004;
        const VOLUME   = 0x0008;
        const DIRECTORY   = 0x0010;
        ///This file has been touched since the last DOS backup was performed on it.
        const ARCHIVE  = 0x0020;
    }
}

impl FatAttr {
    /* Convert attribute bits and a mask to the UNIX mode. */
    fn make_mode(&self, mount_option: ExfatMountOptions, mode: InodeMode) -> InodeMode {
        let mut ret = mode;
        if self.contains(FatAttr::READONLY) && !self.contains(FatAttr::DIRECTORY) {
            ret.remove(InodeMode::S_IWGRP | InodeMode::S_IWUSR | InodeMode::S_IWUSR);
        }
        if self.contains(FatAttr::DIRECTORY) {
            ret.remove(InodeMode::from_bits_truncate(mount_option.fs_dmask));
        } else {
            ret.remove(InodeMode::from_bits_truncate(mount_option.fs_fmask));
        }
        ret
    }
}

#[derive(Debug)]
pub struct ExfatInode(RwLock<ExfatInodeInner>);

impl ExfatInode {
    pub(super) fn default() -> Self {
        ExfatInode(RwLock::new(ExfatInodeInner::default()))
    }

    pub(super) fn hash_index(&self) -> usize {
        self.0.read().hash_index()
    }

    fn count_num_subdir(start_chain: ExfatChain, size: usize) -> Result<usize> {
        if !start_chain.is_current_cluster_valid() {
            return Ok(0);
        }
        //FIXME:What if there are some invalid dentries in volume?
        let iterator = ExfatDentryIterator::new(start_chain, 0, Some(size))?;
        let mut cnt = 0;
        for dentry_result in iterator {
            let dentry = dentry_result?;
            if let ExfatDentry::File(_) = dentry {
                cnt += 1
            }
        }
        Ok(cnt)
    }

    pub(super) fn build_root_inode(fs_weak: Weak<ExfatFS>) -> Result<Arc<ExfatInode>> {
        let sb = fs_weak.upgrade().unwrap().super_block();

        let root_cluster = sb.root_dir;

        let dentry_set_size = 0;

        let attr = FatAttr::DIRECTORY;

        let inode_type = InodeType::Dir;

        let ctime =
            DosTimestamp::from_duration(now_as_duration(&crate::time::ClockID::CLOCK_REALTIME)?)?;

        let start_chain =
            ExfatChain::new(fs_weak.clone(), root_cluster, FatChainFlags::ALLOC_POSSIBLE);

        let size = start_chain.count_clusters()? as usize * sb.cluster_size as usize;

        let name = ExfatName::new();

        let num_subdir = Self::count_num_subdir(start_chain.clone(), size)? as u32;

        Ok(Arc::new_cyclic(|weak_self| {
            ExfatInode(RwLock::new(ExfatInodeInner {
                ino: EXFAT_ROOT_INO,
                dentry_set_position: ExfatChainPosition::default(),
                dentry_set_size: 0,
                dentry_entry: 0,
                inode_type,
                attr,
                start_chain,
                size,
                size_allocated: size,
                atime: ctime,
                mtime: ctime,
                ctime,
                num_subdir,
                name,
                page_cache: PageCache::with_capacity(size, weak_self.clone() as _).unwrap(),
                fs: fs_weak,
            }))
        }))
    }

    fn build_inode_from_dentry_set(
        fs: Arc<ExfatFS>,
        dentry_set: &ExfatDentrySet,
        dentry_set_position: ExfatChainPosition,
        dentry_entry: u32,
        ino: Ino,
    ) -> Result<Arc<ExfatInode>> {
        const EXFAT_MIMIMUM_DENTRY: usize = 3;

        if dentry_set.len() < EXFAT_MIMIMUM_DENTRY {
            return_errno_with_message!(Errno::EINVAL, "invalid dentry length")
        }

        let dentry_set_size = dentry_set.len() * DENTRY_SIZE;

        let fs_weak = Arc::downgrade(&fs);

        let file = dentry_set.get_file_dentry();
        let attr = FatAttr::from_bits_truncate(file.attribute);

        let inode_type = if attr.contains(FatAttr::DIRECTORY) {
            InodeType::Dir
        } else {
            InodeType::File
        };

        let ctime = DosTimestamp::new(
            file.create_time,
            file.create_date,
            file.create_time_cs,
            file.create_utc_offset,
        )?;
        let mtime = DosTimestamp::new(
            file.modify_time,
            file.modify_date,
            file.modify_time_cs,
            file.modify_utc_offset,
        )?;
        let atime = DosTimestamp::new(
            file.access_time,
            file.access_date,
            0,
            file.access_utc_offset,
        )?;

        let stream = dentry_set.get_stream_dentry();
        let size = stream.valid_size as usize;
        let size_allocated = stream.size as usize;

        if attr.contains(FatAttr::DIRECTORY) && size != size_allocated {
            return_errno_with_message!(
                Errno::EINVAL,
                "allocated_size and valid_size can only be different for files!"
            )
        }

        let chain_flag = FatChainFlags::from_bits_truncate(stream.flags);
        let start_cluster = stream.start_cluster;
        let start_chain = ExfatChain::new(fs_weak.clone(), start_cluster, chain_flag);

        let name = dentry_set.get_name()?;

        let num_subdir = if matches!(inode_type, InodeType::File) {
            0
        } else {
            Self::count_num_subdir(start_chain.clone(), size)? as u32
        };

        Ok(Arc::new_cyclic(|weak_self| {
            ExfatInode(RwLock::new(ExfatInodeInner {
                ino,
                dentry_set_position,
                dentry_set_size,
                dentry_entry,
                inode_type,
                attr,
                start_chain,
                size,
                size_allocated,
                atime,
                mtime,
                ctime,
                num_subdir,
                name,
                page_cache: PageCache::with_capacity(size, weak_self.clone() as _).unwrap(),
                fs: fs_weak,
            }))
        }))
    }

    //The caller of the function should give a unique ino to assign to the inode.
    pub(super) fn read_from(
        fs: Arc<ExfatFS>,
        dentry_set_position: ExfatChainPosition,
        dentry_entry: u32,
        ino: Ino,
    ) -> Result<Arc<Self>> {
        let fs_weak = Arc::downgrade(&fs);
        let mut chain = dentry_set_position.0;
        let mut offset = dentry_set_position.1;
        let mut iter = ExfatDentryIterator::new(chain.clone(), offset, None)?;
        let mut entry = dentry_entry;
        //We need to skip empty or deleted dentery.
        loop {
            let dentry_result = iter.next();

            if dentry_result.is_none() {
                return_errno_with_message!(Errno::ENOENT, "inode data not available")
            }

            let dentry = dentry_result.unwrap()?;

            if let ExfatDentry::File(file) = dentry {
                break;
            }
            (chain, offset) = iter.chain_and_offset();
            entry += 1;
        }

        let skiped_dentry_position = (chain, offset);

        let dentry_set = ExfatDentrySet::read_from(&skiped_dentry_position)?;
        Self::build_inode_from_dentry_set(fs, &dentry_set, skiped_dentry_position, entry, ino)
    }
}

//In-memory rust object that represents a file or folder.
#[derive(Debug)]
pub struct ExfatInodeInner {
    ino: Ino,

    dentry_set_position: ExfatChainPosition,
    dentry_set_size: usize,

    //The entry number of the dentry
    dentry_entry: u32,

    inode_type: InodeType,

    attr: FatAttr,

    //Can be a free cluster
    start_chain: ExfatChain,

    //valid size of the file
    size: usize,
    //allocated size, for directory, size is always equal to size_allocated.
    size_allocated: usize,

    //Updated after reading
    atime: DosTimestamp,
    //Updated only on write
    mtime: DosTimestamp,

    ctime: DosTimestamp,

    //Number of sub directories
    num_subdir: u32,

    //exFAT uses UTF-16 encoding, rust use utf-8 for string processing.
    name: ExfatName,

    page_cache: PageCache,

    fs: Weak<ExfatFS>,
}

impl ExfatInodeInner {
    pub(super) fn default() -> Self {
        Self {
            ino: 0,
            dentry_set_position: (ExfatChain::default(), 0),
            dentry_set_size: 0,
            dentry_entry: 0,
            inode_type: InodeType::File,
            attr: FatAttr::empty(),
            start_chain: ExfatChain::default(),
            size: 0,
            size_allocated: 0,

            atime: DosTimestamp::default(),
            mtime: DosTimestamp::default(),
            ctime: DosTimestamp::default(),
            num_subdir: 0,
            name: ExfatName::default(),
            page_cache: PageCache::new(Weak::<ExfatInode>::default()).unwrap(),
            fs: Weak::default(),
        }
    }

    pub fn fs(&self) -> Arc<ExfatFS> {
        self.fs.upgrade().unwrap()
    }

    fn cluster_position_hash(pos: &ExfatChainPosition) -> usize {
        (pos.0.cluster_id() as usize) << 32usize | (pos.1 & 0xffffffffusize)
    }

    pub fn hash_index(&self) -> usize {
        if self.ino == EXFAT_ROOT_INO {
            return ROOT_INODE_HASH;
        }

        Self::cluster_position_hash(&self.dentry_set_position)
    }

    pub fn make_mode(&self) -> InodeMode {
        self.attr
            .make_mode(self.fs().mount_option(), InodeMode::all())
    }

    //Should lock the file system before calling this function.
    fn write_inode(&mut self, sync: bool) -> Result<()> {
        // Root dir should not be updated.
        if self.ino == EXFAT_ROOT_INO {
            return Ok(());
        }

        //If the inode is already unlinked, there is no need for updating it.
        if !self.dentry_set_position.0.is_current_cluster_valid() {
            return Ok(());
        }

        //Need to read the latest dentry set.
        let mut dentry_set = ExfatDentrySet::read_from(&self.dentry_set_position)?;

        let mut file_dentry = dentry_set.get_file_dentry();
        let mut stream_dentry = dentry_set.get_stream_dentry();

        file_dentry.attribute = self.attr.bits();

        file_dentry.create_utc_offset = self.ctime.utc_offset;
        file_dentry.create_date = self.ctime.date;
        file_dentry.create_time = self.ctime.time;
        file_dentry.create_time_cs = self.ctime.increament_10ms;

        file_dentry.modify_utc_offset = self.mtime.utc_offset;
        file_dentry.modify_date = self.mtime.date;
        file_dentry.modify_time = self.mtime.time;
        file_dentry.modify_time_cs = self.mtime.increament_10ms;

        file_dentry.access_utc_offset = self.atime.utc_offset;
        file_dentry.access_date = self.atime.date;
        file_dentry.access_time = self.atime.time;

        if !self.start_chain.is_current_cluster_valid() {
            self.size = 0;
            self.size_allocated = 0;
        }

        stream_dentry.valid_size = self.size as u64;
        stream_dentry.size = self.size_allocated as u64;
        stream_dentry.start_cluster = self.start_chain.cluster_id();
        stream_dentry.flags = self.start_chain.flags().bits();

        dentry_set.set_file_dentry(&file_dentry);
        dentry_set.set_stream_dentry(&stream_dentry);
        dentry_set.update_checksum();

        dentry_set.write_at(&self.dentry_set_position)?;

        Ok(())
    }

    // allocate clusters in fat mode, return the first allocated cluster id. Bitmap need to be already locked.
    fn alloc_cluster_fat(
        &mut self,
        num_to_be_allocated: u32,
        sync_bitmap: bool,
        bitmap: &mut SpinLockGuard<'_, ExfatBitmap>,
    ) -> Result<ClusterID> {
        let fs = self.fs();

        let sb = fs.super_block();
        let mut alloc_start_cluster = 0;
        let mut prev_cluster = 0;
        let mut cur_cluster = EXFAT_FIRST_CLUSTER;
        for i in 0..num_to_be_allocated {
            cur_cluster = bitmap.find_next_free_cluster(cur_cluster)?;
            bitmap.set_bitmap_used(cur_cluster, sync_bitmap)?;
            if i == 0 {
                alloc_start_cluster = cur_cluster;
            } else {
                fs.write_next_fat(prev_cluster, FatValue::Next(cur_cluster))?;
            }
            prev_cluster = cur_cluster;
        }
        fs.write_next_fat(prev_cluster, FatValue::EndOfChain)?;
        Ok(alloc_start_cluster)
    }

    fn free_cluster_fat(
        &mut self,
        start_cluster: ClusterID,
        free_num: u32,
        sync_bitmap: bool,
        bitmap: &mut SpinLockGuard<'_, ExfatBitmap>,
    ) -> Result<()> {
        let fs = self.fs();

        let mut cur_cluster = start_cluster;
        for i in 0..free_num {
            bitmap.set_bitmap_unused(cur_cluster, sync_bitmap)?;
            match fs.read_next_fat(cur_cluster)? {
                FatValue::Next(data) => {
                    cur_cluster = data;
                }
                _ => return_errno_with_message!(Errno::EINVAL, "invalid fat entry"),
            }
        }

        Ok(())
    }

    //Get the physical cluster id from the logical cluster id in the inode.
    fn get_physical_cluster(&self, logical: ClusterID) -> Result<ClusterID> {
        let chain = self.start_chain.walk(logical)?;
        Ok(chain.cluster_id())
    }

    fn num_clusters(&self) -> ClusterID {
        let cluster_size = self.fs().cluster_size();
        (self.size_allocated.align_up(cluster_size) / cluster_size) as u32
    }

    fn fat_in_use(&self) -> bool {
        !self
            .start_chain
            .flags()
            .contains(FatChainFlags::FAT_CHAIN_NOT_IN_USE)
    }

    //Get the cluster id from the logical cluster id in the inode. If cluster not exist, allocate a new one.
    //exFAT do not support holes in the file, so new clusters need to be allocated.
    //The file system must be locked before calling.
    fn get_or_allocate_cluster(&mut self, logical: ClusterID, sync: bool) -> Result<ClusterID> {
        let num_cluster = self.num_clusters();
        if logical >= num_cluster {
            self.alloc_cluster(logical - num_cluster + 1, sync)?;
        }

        self.get_physical_cluster(logical)
    }

    // If current capacity is 0(no start_cluster), this means we can choose a allocation type
    // We first try continuous allocation
    // If no continuous allocation available, turn to fat allocation
    fn alloc_cluster_from_empty(
        &mut self,
        num_to_be_allocated: u32,
        bitmap: &mut SpinLockGuard<'_, ExfatBitmap>,
        sync_bitmap: bool,
    ) -> Result<ClusterID> {
        // search for a continuous chunk big enough
        let search_result =
            bitmap.find_next_free_cluster_range_fast(EXFAT_FIRST_CLUSTER, num_to_be_allocated);
        match search_result {
            Ok(clusters) => {
                let allocated_start_cluster = clusters.start;
                bitmap.set_bitmap_range_used(clusters, sync_bitmap)?;
                self.start_chain = ExfatChain::new(
                    self.fs.clone(),
                    allocated_start_cluster,
                    FatChainFlags::FAT_CHAIN_NOT_IN_USE,
                );
                Ok(allocated_start_cluster)
            }
            _ => {
                // no continuous chunk available, use fat table
                let allocated_start_cluster =
                    self.alloc_cluster_fat(num_to_be_allocated, sync_bitmap, bitmap)?;
                self.start_chain = ExfatChain::new(
                    self.fs.clone(),
                    allocated_start_cluster,
                    FatChainFlags::ALLOC_POSSIBLE,
                );
                Ok(allocated_start_cluster)
            }
        }
    }

    // Append clusters at the end of file, return the first allocated cluster
    // Caller should update size_allocated accordingly.
    // The file system must be locked before calling.
    fn alloc_cluster(&mut self, num_to_be_allocated: u32, sync_bitmap: bool) -> Result<ClusterID> {
        info!("Begin to allocate {} clusters", num_to_be_allocated);
        let fs = self.fs();

        let bitmap_binding = fs.bitmap();
        let mut bitmap = bitmap_binding.lock();
        if num_to_be_allocated > bitmap.free_clusters() {
            return_errno!(Errno::ENOSPC)
        }

        let num_clusters = self.num_clusters();
        if num_clusters == 0 {
            return self.alloc_cluster_from_empty(num_to_be_allocated, &mut bitmap, sync_bitmap);
        }

        let start_cluster = self.start_chain.cluster_id();

        // Try to alloc contiguously otherwise break the chain.
        if !self.fat_in_use() {
            // first, check if there are enough following clusters.
            // if not, we can give up continuous allocation and turn to fat allocation
            let current_end = start_cluster + num_clusters;
            let clusters = current_end..(current_end + num_to_be_allocated);
            let check_result = bitmap.is_cluster_range_free(clusters.clone());
            if check_result.is_ok() && check_result.unwrap() {
                // Considering that the following clusters may be out of range, we should deal with this error here(just turn to fat allocation)
                bitmap.set_bitmap_range_used(clusters, sync_bitmap)?;
                return Ok(start_cluster);
            } else {
                // break the chain.
                for i in 0..num_clusters - 1 {
                    fs.write_next_fat(start_cluster + i, FatValue::Next(start_cluster + i + 1))?;
                }
                fs.write_next_fat(start_cluster + num_clusters - 1, FatValue::EndOfChain)?;
                self.start_chain.set_flags(FatChainFlags::ALLOC_POSSIBLE);
            }
        }

        //Allocate remaining clusters the tail.
        let allocated_start_cluster =
            self.alloc_cluster_fat(num_to_be_allocated, sync_bitmap, &mut bitmap)?;

        //Insert allocated clusters to the tail.
        let tail_cluster = self.get_physical_cluster(num_clusters - 1)?;
        info!(
            "Write tail for allocated cluster. tail:{}, next_allocated:{}",
            tail_cluster, allocated_start_cluster
        );
        fs.write_next_fat(tail_cluster, FatValue::Next(allocated_start_cluster))?;

        Ok(allocated_start_cluster)
    }

    // free physical clusters start from "start_cluster", number is "free_num", allocation mode is "flags"
    // this function not related to specific inode
    fn free_cluster_range(
        &mut self,
        physical_start_cluster: ClusterID,
        free_num: u32,
        sync_bitmap: bool,
    ) -> Result<()> {
        let fs = self.fs();

        let bitmap_binding = fs.bitmap();
        let mut bitmap = bitmap_binding.lock();

        if !self.fat_in_use() {
            bitmap.set_bitmap_range_unused(
                physical_start_cluster..physical_start_cluster + free_num,
                sync_bitmap,
            )?;
        } else {
            self.free_cluster_fat(physical_start_cluster, free_num, sync_bitmap, &mut bitmap)?;
        }
        Ok(())
    }

    // free the tailing clusters in this inode, the number is free_num
    fn free_tailing_cluster(&mut self, free_num: u32, sync_bitmap: bool) -> Result<()> {
        let num_clusters = self.num_clusters();
        let logical_trunc_start_cluster = num_clusters - free_num;

        let physical_trunc_start_cluster =
            self.get_physical_cluster(logical_trunc_start_cluster)?;

        self.free_cluster_range(physical_trunc_start_cluster, free_num, sync_bitmap)?;

        if self.fat_in_use() && logical_trunc_start_cluster != 0 {
            let end_cluster = self.get_physical_cluster(logical_trunc_start_cluster - 1)?;
            self.fs()
                .write_next_fat(end_cluster, FatValue::EndOfChain)?;
        }

        Ok(())
    }

    pub fn resize(&mut self, new_size: usize) -> Result<()> {
        let fs = self.fs();
        let cluster_size = fs.cluster_size();
        let num_clusters = self.num_clusters();

        let new_num_clusters = (new_size.align_up(cluster_size) / cluster_size) as u32;

        let sync = self.is_sync();

        match new_num_clusters.cmp(&num_clusters) {
            Ordering::Greater => {
                self.alloc_cluster(new_num_clusters - num_clusters, sync)?;
            }
            Ordering::Less => {
                self.free_tailing_cluster(num_clusters - new_num_clusters, sync)?;
                if new_size < self.size {
                    //Valid data is truncated.
                    self.size = new_size;
                }
            }
            _ => {}
        };
        self.size_allocated = new_size;

        if sync {
            self.write_inode(true)?;
        }

        Ok(())
    }

    pub fn lock_and_resize(&mut self, new_size: usize) -> Result<()> {
        let fs = self.fs();
        let guard = fs.lock();
        self.resize(new_size)
    }

    fn get_sector_id(&self, sector_id: usize) -> Result<usize> {
        let chain_offset = self
            .start_chain
            .walk_to_cluster_at_offset(sector_id * self.fs().sector_size())?;

        let sect_per_cluster = self.fs().super_block().sect_per_cluster as usize;
        let cluster_id = sector_id / sect_per_cluster;
        let cluster = self.get_physical_cluster((sector_id / sect_per_cluster) as ClusterID)?;

        let sec_offset = sector_id % (self.fs().super_block().sect_per_cluster as usize);
        Ok(self.fs().cluster_to_off(cluster) / self.fs().sector_size() + sec_offset)
    }

    //Get the physical sector id from the logical sector id in the inode.
    fn lock_and_get_or_allocate_sector_id(
        &mut self,
        sector_id: usize,
        alloc_page: bool,
    ) -> Result<usize> {
        let binding = self.fs();
        let guard = binding.lock();

        let sector_size = self.fs().sector_size();

        let sector_per_page = PAGE_SIZE / sector_size;
        let mut sector_end = sector_id;
        if alloc_page {
            sector_end = sector_id.align_up(sector_per_page);
        }

        let last_sector = self.size_allocated / sector_size;

        let sync = self.is_sync();
        //Cluster size must be larger than page_size.
        let cluster = self.get_or_allocate_cluster(
            sector_end as u32 / self.fs().super_block().sect_per_cluster,
            sync,
        )?;

        let sec_offset = sector_id % (self.fs().super_block().sect_per_cluster as usize);

        Ok(self.fs().cluster_to_off(cluster) / sector_size + sec_offset)
    }

    fn page_cache(&self) -> &PageCache {
        &self.page_cache
    }

    fn read_sector(&self, idx: usize, frame: &VmFrame) -> Result<()> {
        let sector_id = self.get_sector_id(idx)?;
        self.fs().block_device().read_sector(sector_id, frame)?;
        Ok(())
    }

    fn write_sector(&mut self, idx: usize, frame: &VmFrame) -> Result<()> {
        let sector_id = self.lock_and_get_or_allocate_sector_id(idx, false)?;
        self.fs().block_device().write_sector(sector_id, frame)?;
        Ok(())
    }

    fn is_sync(&self) -> bool {
        true
        //TODO:Judge whether sync is necessary.
    }

    //Find empty dentry. If not found, expand the clusterchain.
    fn find_empty_dentry(&mut self, num_dentries: usize) -> Result<usize> {
        let fs = self.fs();
        let dentry_iterator =
            ExfatDentryIterator::new(self.start_chain.clone(), 0, Some(self.size))?;

        let mut cont_unused = 0;
        let mut entry_id = 0;

        for dentry_result in dentry_iterator {
            let dentry = dentry_result?;
            match dentry {
                ExfatDentry::UnUsed | ExfatDentry::Deleted(_) => {
                    cont_unused += 1;
                }
                _ => {
                    cont_unused = 0;
                }
            }
            if cont_unused >= num_dentries {
                return Ok(entry_id - (num_dentries - 1));
            }
            entry_id += 1;
        }

        //Empty entries not found, allocate new cluster
        if self.size >= MAX_EXFAT_DENTRIES as usize * DENTRY_SIZE {
            return_errno!(Errno::ENOSPC)
        }

        let cluster_size = self.fs().cluster_size();
        let cluster_to_be_allocated =
            (num_dentries * DENTRY_SIZE).align_up(cluster_size) / cluster_size;

        self.alloc_cluster(cluster_to_be_allocated as u32, self.is_sync())?;
        self.size_allocated += cluster_size * cluster_to_be_allocated;
        self.size = self.size_allocated;

        Ok(entry_id)
    }

    fn add_entry(
        &mut self,
        name: &str,
        inode_type: InodeType,
        mode: InodeMode,
    ) -> Result<Arc<ExfatInode>> {
        if name.len() > MAX_NAME_LENGTH {
            return_errno!(Errno::ENAMETOOLONG)
        }

        //Allocate new cluster if no cluster is associated.
        if !self.start_chain.is_current_cluster_valid() {
            self.alloc_cluster(1, self.is_sync())?;
        }

        //TODO: remove trailing periods of pathname.
        //Do not allow creation of files with names ending with period(s).

        let name_dentries = (name.len() + EXFAT_FILE_NAME_LEN - 1) / EXFAT_FILE_NAME_LEN;
        let num_dentries = name_dentries + 2; // FILE Entry + Stream Entry + Name Entry
        let entry = self.find_empty_dentry(num_dentries)? as u32;

        if inode_type.is_directory() && !self.fs().mount_option().zero_size_dir {
            //We need to resize the directory so that it contains at least 1 cluster if zero_size_dir is not enabled.
        }

        let dentry_set = ExfatDentrySet::from(self.fs().clone(), name, inode_type, mode)?;
        let pos = self
            .start_chain
            .walk_to_cluster_at_offset(entry as usize * DENTRY_SIZE)?;

        dentry_set.write_at(&pos)?;

        self.num_subdir += 1;

        ExfatInode::build_inode_from_dentry_set(
            self.fs(),
            &dentry_set,
            pos,
            entry,
            self.fs().alloc_inode_number(),
        )
    }

    fn read_single_dir(
        &self,
        offset: usize,
        visitor: &mut dyn DirentVisitor,
    ) -> Result<(Arc<ExfatInode>, usize)> {
        if !self.inode_type.is_directory() {
            return_errno!(Errno::ENOTDIR)
        }
        let fs = self.fs();
        let cluster_size = fs.cluster_size();

        let physical_cluster = self.get_physical_cluster((offset / cluster_size) as u32)?;
        let cluster_off = (offset % cluster_size) as u32;

        let ino = fs.alloc_inode_number();
        let dentry_position_start = self.start_chain.walk_to_cluster_at_offset(offset)?;
        let child_inode = ExfatInode::read_from(
            fs.clone(),
            dentry_position_start,
            (offset / DENTRY_SIZE) as u32,
            ino,
        )?;
        let dentry_position = child_inode.0.read().dentry_set_position.clone();

        if let Some(inode) = fs.find_opened_inode(make_hash_index(
            dentry_position.0.cluster_id(),
            dentry_position.1 as u32,
        )) {
            let child_inner = inode.0.read();
            visitor.visit(
                &child_inner.name.to_string(),
                child_inner.ino as u64,
                child_inner.inode_type,
                offset,
            )?;

            Ok((
                inode.clone(),
                child_inner.dentry_entry as usize * DENTRY_SIZE + child_inner.dentry_set_size,
            ))
        } else {
            let _ = fs.insert_inode(child_inode.clone());
            let child_inner = child_inode.0.read();
            visitor.visit(
                &child_inner.name.to_string(),
                ino as u64,
                child_inner.inode_type,
                offset,
            )?;
            Ok((
                child_inode.clone(),
                child_inner.dentry_entry as usize * DENTRY_SIZE + child_inner.dentry_set_size,
            ))
        }
    }

    // look up a target with "name", cur inode represent a dir
    // return (target inode, dentries start offset, dentries len)
    // No inode should hold the write lock.
    fn lookup_by_name(&self, name: &str) -> Result<(Arc<ExfatInode>, usize, usize)> {
        let sub_dir = self.num_subdir;
        let mut names: Vec<String> = vec![];
        let mut offset = 0;

        for i in 0..sub_dir {
            let (inode, next) = self.read_single_dir(offset, &mut names)?;

            if names.last().unwrap().eq(name) {
                let pos = inode.0.read().dentry_entry;
                let size = inode.0.read().dentry_set_size;
                return Ok((inode, pos as usize * DENTRY_SIZE, size));
            }
            offset = next;
        }
        return_errno!(Errno::ENOENT)
    }

    // only valid for directory, check if the dir is empty
    fn is_empty_dir(&self) -> Result<bool> {
        if !self.inode_type.is_directory() {
            return_errno!(Errno::ENOTDIR)
        }
        let iterator = ExfatDentryIterator::new(self.start_chain.clone(), 0, Some(self.size))?;

        for dentry_result in iterator {
            let dentry = dentry_result?;
            match dentry {
                ExfatDentry::UnUsed => {}
                ExfatDentry::Deleted(_) => {}
                _ => {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    // delete dentries for cur dir
    fn delete_dentry_set(&mut self, offset: usize, len: usize) -> Result<()> {
        let mut buf = vec![0; len];
        self.start_chain.read_at(offset, &mut buf)?;

        let num_dentry = len / DENTRY_SIZE;

        let cluster_size = self.fs().super_block().cluster_size as usize;
        for i in 0..num_dentry {
            let buf_offset = DENTRY_SIZE * i;
            // delete cluster chain if needed
            let dentry = ExfatDentry::try_from(&buf[buf_offset..buf_offset + DENTRY_SIZE])?;
            match dentry {
                ExfatDentry::VendorAlloc(inner) => {
                    if !self.fs().is_valid_cluster(inner.start_cluster) {
                        continue;
                    }
                    let num_to_free = (inner.size as usize / cluster_size) as u32;
                    self.free_cluster_range(inner.start_cluster, num_to_free, self.is_sync())?;
                }
                ExfatDentry::GenericSecondary(inner) => {
                    if !self.fs().is_valid_cluster(inner.start_cluster) {
                        continue;
                    }
                    let num_to_free = (inner.size as usize / cluster_size) as u32;
                    self.free_cluster_range(inner.start_cluster, num_to_free, self.is_sync())?;
                }
                _ => {}
            }

            // mark this dentry deleted
            buf[buf_offset] &= 0x7F;
        }
        self.start_chain.write_at(offset, &buf)?;
        
        self.num_subdir -= 1;
        // FIXME: We must make sure that there are no spare tailing clusters in a directory.
        Ok(())
    }
}

impl Inode for ExfatInode {
    fn len(&self) -> usize {
        self.0.read().size
    }

    fn resize(&self, new_size: usize) {
        //FIXME: how to return error?
        let mut inner = self.0.write();
        let _ = inner.lock_and_resize(new_size);
    }

    fn metadata(&self) -> crate::fs::utils::Metadata {
        let inner = self.0.read();
        let blk_size = inner.fs().super_block().sector_size as usize;
        Metadata {
            dev: 0,
            ino: inner.ino,
            size: inner.size,
            blk_size,
            blocks: (inner.size + blk_size - 1) / blk_size,
            atime: inner.atime.as_duration().unwrap_or_default(),
            mtime: inner.mtime.as_duration().unwrap_or_default(),
            ctime: inner.ctime.as_duration().unwrap_or_default(),
            type_: inner.inode_type,
            mode: inner.make_mode(),
            nlinks: inner.num_subdir as usize,
            uid: inner.fs().mount_option().fs_uid,
            gid: inner.fs().mount_option().fs_gid,
            //real device
            rdev: 0,
        }
    }

    fn type_(&self) -> InodeType {
        self.0.read().inode_type
    }

    fn mode(&self) -> InodeMode {
        self.0.read().make_mode()
    }

    fn set_mode(&self, mode: InodeMode) {
        //Mode Set not supported.
        todo!("Set inode to readonly")
    }

    fn atime(&self) -> Duration {
        self.0.read().atime.as_duration().unwrap_or_default()
    }

    fn set_atime(&self, time: Duration) {
        self.0.write().atime = DosTimestamp::from_duration(time).unwrap_or_default();
    }

    fn mtime(&self) -> Duration {
        self.0.read().mtime.as_duration().unwrap_or_default()
    }

    fn set_mtime(&self, time: Duration) {
        self.0.write().mtime = DosTimestamp::from_duration(time).unwrap_or_default();
    }

    fn fs(&self) -> alloc::sync::Arc<dyn crate::fs::utils::FileSystem> {
        self.0.read().fs()
    }

    fn read_page(&self, idx: usize, frame: &VmFrame) -> Result<()> {
        let inner = self.0.read();
        if inner.size < idx * PAGE_SIZE {
            return_errno_with_message!(Errno::EINVAL, "Invalid read size")
        }
        let sector_id = inner.get_sector_id(idx * PAGE_SIZE / inner.fs().sector_size())?;
        inner
            .fs()
            .block_device()
            .read_page(sector_id * inner.fs().sector_size() / PAGE_SIZE, frame)?;
        Ok(())
    }

    //What if block_size is not equal to page size?
    fn write_page(&self, idx: usize, frame: &VmFrame) -> Result<()> {
        let inner = self.0.read();
        let sector_size = inner.fs().sector_size();
        //FIXME: dead lock may occur here.

        let sector_id = inner.get_sector_id(idx * PAGE_SIZE / SECTOR_SIZE)?;

        //FIXME: We may need to truncate the file if write_page fails?
        inner
            .fs()
            .block_device()
            .write_page(sector_id * inner.fs().sector_size() / PAGE_SIZE, frame)?;

        Ok(())
    }

    fn page_cache(&self) -> Option<Vmo<Full>> {
        Some(self.0.read().page_cache().pages())
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        let inner = self.0.read();
        if inner.inode_type.is_directory() {
            return_errno!(Errno::EISDIR)
        }
        let (off, read_len) = {
            let file_size = inner.size;
            let start = file_size.min(offset);
            let end = file_size.min(offset + buf.len());
            (start, end - start)
        };
        inner
            .page_cache
            .pages()
            .read_bytes(offset, &mut buf[..read_len])?;

        Ok(read_len)
    }

    // The offset and the length of buffer must be multiples of the block size.
    fn read_direct_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        let inner = self.0.read();
        if inner.inode_type.is_directory() {
            return_errno!(Errno::EISDIR)
        }
        if !is_block_aligned(offset) || !is_block_aligned(buf.len()) {
            return_errno_with_message!(Errno::EINVAL, "not block-aligned");
        }

        let sector_size = inner.fs().sector_size();

        let (offset, read_len) = {
            let file_size = inner.size;
            let start = file_size.min(offset).align_down(sector_size);
            let end = file_size.min(offset + buf.len()).align_down(sector_size);
            (start, end - start)
        };

        inner
            .page_cache()
            .pages()
            .decommit(offset..offset + read_len)?;

        let mut buf_offset = 0;
        let frame = VmAllocOptions::new(1).uninit(true).alloc_single().unwrap();

        for bid in offset / sector_size..(offset + read_len) / sector_size {
            inner.read_sector(bid, &frame)?;
            frame.read_bytes(0, &mut buf[buf_offset..buf_offset + sector_size])?;
            buf_offset += sector_size;
        }
        Ok(read_len)
    }

    fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        //We need to obtain the lock to resize the file.
        {
            let mut inner = self.0.write();
            if inner.inode_type.is_directory() {
                return_errno!(Errno::EISDIR)
            }

            let file_size = inner.size;
            let new_size = offset + buf.len();

            if new_size > file_size {
                inner.lock_and_resize(new_size)?;
                inner.page_cache.pages().resize(new_size)?;

                inner.size = new_size;
                //TODO:We need to fill the page cache with 0.
            }
        }

        //Lock released here.

        self.0.read().page_cache.pages().write_bytes(offset, buf)?;

        Ok(buf.len())
    }

    fn write_direct_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        let mut inner = self.0.write();
        if inner.inode_type.is_directory() {
            return_errno!(Errno::EISDIR)
        }
        if !is_block_aligned(offset) || !is_block_aligned(buf.len()) {
            return_errno_with_message!(Errno::EINVAL, "not block-aligned");
        }

        let file_size = inner.size;
        let end_offset = offset + buf.len();

        let start = offset.min(file_size);
        let end = end_offset.min(file_size);
        inner.page_cache.pages().decommit(start..end)?;

        if end_offset > file_size {
            inner.page_cache.pages().resize(end_offset)?;
            inner.lock_and_resize(end_offset)?;
            inner.size = end_offset;
        }

        //TODO: We need to write 0 to extented space.

        let block_size = inner.fs().sector_size();

        let mut buf_offset = 0;
        for bid in offset / block_size..(end_offset) / block_size {
            let frame = {
                let frame = VmAllocOptions::new(1).uninit(true).alloc_single().unwrap();
                frame.write_bytes(0, &buf[buf_offset..buf_offset + block_size])?;
                frame
            };
            inner.write_sector(bid, &frame)?;
            buf_offset += block_size;
        }

        Ok(buf_offset)
    }

    fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>> {
        let mut inner = self.0.write();
        if !inner.inode_type.is_directory() {
            return_errno!(Errno::ENOTDIR)
        }
        if name.len() > MAX_NAME_LENGTH {
            return_errno!(Errno::ENAMETOOLONG)
        }

        let fs = inner.fs();
        let guard = fs.lock();

        if inner.lookup_by_name(name).is_ok() {
            return_errno!(Errno::EEXIST)
        }

        let result = inner.add_entry(name, type_, mode)?;
        let _ = fs.insert_inode(result.clone());
        Ok(result)
    }

    fn mknod(&self, name: &str, mode: InodeMode, dev: Arc<dyn Device>) -> Result<Arc<dyn Inode>> {
        return_errno_with_message!(Errno::EINVAL, "unsupported operation")
    }

    fn readdir_at(&self, dir_cnt: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        let inner = self.0.read();

        if dir_cnt >= inner.num_subdir as usize {
            return Ok(0);
        }

        struct EmptyVistor;
        impl DirentVisitor for EmptyVistor {
            fn visit(
                &mut self,
                name: &str,
                ino: u64,
                type_: InodeType,
                offset: usize,
            ) -> Result<()> {
                Ok(())
            }
        }
        let mut empty_visitor = EmptyVistor;

        let mut offset = 0;

        let fs = inner.fs();
        let guard = fs.lock();

        //Skip previous directories.
        for i in 0..dir_cnt {
            let (_, next_offset) = inner.read_single_dir(offset, &mut empty_visitor)?;
            offset = next_offset;
        }

        for i in dir_cnt as u32..inner.num_subdir {
            let (_, next_offset) = inner.read_single_dir(offset, visitor)?;
            offset = next_offset;
        }
        Ok(inner.num_subdir as usize)
    }

    fn link(&self, old: &Arc<dyn Inode>, name: &str) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL, "unsupported operation")
    }

    fn unlink(&self, name: &str) -> Result<()> {
        let mut inner = self.0.write();
        if !inner.inode_type.is_directory() {
            return_errno!(Errno::ENOTDIR)
        }
        if name.len() > MAX_NAME_LENGTH {
            return_errno!(Errno::ENAMETOOLONG)
        }
        if name == "." || name == ".." {
            return_errno!(Errno::EISDIR)
        }

        let fs = inner.fs();
        let guard = fs.lock();

        let (inode, offset, len) = inner.lookup_by_name(name)?;
        if inode.type_() != InodeType::File {
            return_errno!(Errno::EISDIR)
        }

        inode.0.write().resize(0)?;

        inner.delete_dentry_set(offset, len)?;
        Ok(())
    }

    fn rmdir(&self, name: &str) -> Result<()> {
        let mut inner = self.0.write();
        if !inner.inode_type.is_directory() {
            return_errno!(Errno::ENOTDIR)
        }
        if name == "." {
            return_errno_with_message!(Errno::EINVAL, "rmdir on .")
        }
        if name == ".." {
            return_errno_with_message!(Errno::ENOTEMPTY, "rmdir on ..")
        }
        if name.len() > MAX_NAME_LENGTH {
            return_errno!(Errno::ENAMETOOLONG)
        }

        let fs = inner.fs();
        let guard = fs.lock();

        let (inode, offset, len) = inner.lookup_by_name(name)?;
        if inode.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR)
        } else if !inode.0.read().is_empty_dir()? {
            // check if directory to be deleted is empty
            return_errno!(Errno::ENOTEMPTY)
        }

        inode.0.write().resize(0)?;

        inner.delete_dentry_set(offset, len)?;
        Ok(())
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        //FIXME: Readdir should be immutable instead of mutable, but there will be no performance issues due to the global fs lock.
        let inner = self.0.read();
        if !inner.inode_type.is_directory() {
            return_errno!(Errno::ENOTDIR)
        }

        if name.len() > MAX_NAME_LENGTH {
            return_errno!(Errno::ENAMETOOLONG)
        }

        let fs = inner.fs();
        let guard = fs.lock();

        let (inode, _, _) = inner.lookup_by_name(name)?;
        Ok(inode)
    }

    fn rename(&self, old_name: &str, target: &Arc<dyn Inode>, new_name: &str) -> Result<()> {
        if old_name == "." || old_name == ".." || new_name == "." || new_name == ".." {
            return_errno!(Errno::EISDIR);
        }
        if old_name.len() > MAX_NAME_LENGTH || new_name.len() > MAX_NAME_LENGTH {
            return_errno!(Errno::ENAMETOOLONG)
        }

        let Some(target_) = target.downcast_ref::<ExfatInode>() else {
            return_errno_with_message!(Errno::EINVAL, "not an exfat inode")
        };

        let fs = self.0.read().fs();
        let guard = fs.lock();

        //There will be no deadlocks since the whole fs is locked.

        if !self.0.read().inode_type.is_directory() || !target_.0.read().inode_type.is_directory() {
            return_errno!(Errno::ENOTDIR)
        }

        // read 'old_name' file or dir and its dentries
        let (old_inode, old_offset, old_len) = self.0.read().lookup_by_name(old_name)?;
        let old_inode_inner = old_inode.0.read();

        let mut exist_inode: Arc<ExfatInode> = ExfatInode::default().into();
        let mut exist_offset: usize = 0;
        let mut exist_len: usize = 0;
        let need_to_remove_exist =
            if let Ok((node, offset, len)) = target_.0.read().lookup_by_name(new_name) {
                exist_inode = node;
                exist_offset = offset;
                exist_len = len;
                // check for a corner case here
                // if 'old_name' represents a directory, the exist 'new_name' must represents a empty directory.
                if old_inode_inner.inode_type.is_directory()
                    && !exist_inode.0.read().is_empty_dir()?
                {
                    return_errno!(Errno::ENOTEMPTY)
                }
                true
            } else {
                false
            };

        // copy the dentries
        let new_inode =
            target_
                .0
                .write()
                .add_entry(new_name, old_inode.type_(), old_inode.mode())?;

        let mut new_inode_inner = new_inode.0.write();
        new_inode_inner.start_chain = ExfatChain::new(
            Arc::downgrade(&fs),
            old_inode_inner.start_chain.cluster_id(),
            old_inode_inner.start_chain.flags(),
        );
        new_inode_inner.size = old_inode_inner.size;
        new_inode_inner.size_allocated = old_inode_inner.size_allocated;
        new_inode_inner.write_inode(true)?;

        // delete 'old_name' dentries
        self.0.write().delete_dentry_set(old_offset, old_len)?;

        // remove the exist 'new_name' file(include file contents and dentries)
        if need_to_remove_exist {
            fs.evice_inode(exist_inode.hash_index());
            exist_inode.0.write().resize(0)?;
            exist_inode
                .0
                .write()
                .delete_dentry_set(exist_offset, exist_len)?;
        }

        Ok(())
    }

    fn read_link(&self) -> Result<String> {
        return_errno_with_message!(Errno::EINVAL, "unsupported operation")
    }

    fn write_link(&self, target: &str) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL, "unsupported operation")
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        todo!()
    }

    fn sync(&self) -> Result<()> {
        self.0.read().page_cache().evict_range(0..self.len())?;

        let mut inner = self.0.write();
        let fs = inner.fs();
        let guard = fs.lock();

        inner.write_inode(true)?;
        Ok(())
    }

    fn poll(&self, mask: IoEvents, _poller: Option<&Poller>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }

    fn is_dentry_cacheable(&self) -> bool {
        true
    }
}
