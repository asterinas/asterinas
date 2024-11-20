// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]
#![allow(unused_variables)]

use alloc::string::String;
use core::{cmp::Ordering, time::Duration};

pub(super) use align_ext::AlignExt;
use aster_block::{
    bio::{BioDirection, BioSegment, BioWaiter},
    id::{Bid, BlockId},
    BLOCK_SIZE,
};
use aster_rights::Full;
use ostd::mm::{Frame, VmIo};

use super::{
    constants::*,
    dentry::{
        Checksum, ExfatDentry, ExfatDentrySet, ExfatFileDentry, ExfatName, RawExfatDentry,
        DENTRY_SIZE,
    },
    fat::{ClusterAllocator, ClusterID, ExfatChainPosition, FatChainFlags},
    fs::{ExfatMountOptions, EXFAT_ROOT_INO},
    utils::{make_hash_index, DosTimestamp},
};
use crate::{
    events::IoEvents,
    fs::{
        exfat::{dentry::ExfatDentryIterator, fat::ExfatChain, fs::ExfatFS},
        utils::{
            DirentVisitor, Extension, Inode, InodeMode, InodeType, IoctlCmd, Metadata, MknodType,
            PageCache, PageCacheBackend,
        },
    },
    prelude::*,
    process::{signal::PollHandle, Gid, Uid},
    vm::vmo::Vmo,
};

///Inode number
pub type Ino = u64;

bitflags! {
    pub struct FatAttr : u16{
        /// This inode is read only.
        const READONLY  = 0x0001;
        /// This inode is hidden. This attribute is not supported in our implementation.
        const HIDDEN    = 0x0002;
        /// This inode belongs to the OS. This attribute is not supported in our implementation.
        const SYSTEM    = 0x0004;
        /// This inode represents a volume. This attribute is not supported in our implementation.
        const VOLUME    = 0x0008;
        /// This inode represents a directory.
        const DIRECTORY = 0x0010;
        /// This file has been touched since the last DOS backup was performed on it. This attribute is not supported in our implementation.
        const ARCHIVE   = 0x0020;
    }
}

impl FatAttr {
    /// Convert attribute bits and a mask to the UNIX mode.
    fn make_mode(&self, mount_option: ExfatMountOptions, mode: InodeMode) -> InodeMode {
        let mut ret = mode;
        if self.contains(FatAttr::READONLY) && !self.contains(FatAttr::DIRECTORY) {
            ret.remove(InodeMode::S_IWGRP | InodeMode::S_IWUSR | InodeMode::S_IWOTH);
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
pub struct ExfatInode {
    inner: RwMutex<ExfatInodeInner>,
    extension: Extension,
}

#[derive(Debug)]
struct ExfatInodeInner {
    /// Inode number.
    ino: Ino,

    /// Dentry set position in its parent directory.
    dentry_set_position: ExfatChainPosition,
    /// Dentry set size in bytes.
    dentry_set_size: usize,
    /// The entry number of the dentry.
    dentry_entry: u32,
    /// Inode type, File or Dir.
    inode_type: InodeType,

    attr: FatAttr,

    /// Start position on disk, this is undefined if the allocated size is 0.
    start_chain: ExfatChain,

    /// Valid size of the file.
    size: usize,
    /// Allocated size, for directory, size is always equal to size_allocated.
    size_allocated: usize,

    /// Access time, updated after reading.
    atime: DosTimestamp,
    /// Modification time, updated only on write.
    mtime: DosTimestamp,
    /// Creation time.
    ctime: DosTimestamp,

    /// Number of sub inodes.
    num_sub_inodes: u32,
    /// Number of sub inodes that are directories.
    num_sub_dirs: u32,

    /// ExFAT uses UTF-16 encoding, rust use utf-8 for string processing.
    name: ExfatName,

    /// Flag for whether the inode is deleted.
    is_deleted: bool,

    /// The hash of its parent inode.
    parent_hash: usize,

    /// A pointer to exFAT fs.
    fs: Weak<ExfatFS>,

    /// Important: To enlarge the page_cache, we need to update the page_cache size before we update the size of inode, to avoid extra data read.
    /// To shrink the page_cache, we need to update the page_cache size after we update the size of inode, to avoid extra data write.
    page_cache: PageCache,
}

impl PageCacheBackend for ExfatInode {
    fn read_page_async(&self, idx: usize, frame: &Frame) -> Result<BioWaiter> {
        let inner = self.inner.read();
        if inner.size < idx * PAGE_SIZE {
            return_errno_with_message!(Errno::EINVAL, "Invalid read size")
        }
        let sector_id = inner.get_sector_id(idx * PAGE_SIZE / inner.fs().sector_size())?;
        let bio_segment =
            BioSegment::new_from_segment(frame.clone().into(), BioDirection::FromDevice);
        let waiter = inner.fs().block_device().read_blocks_async(
            BlockId::from_offset(sector_id * inner.fs().sector_size()),
            bio_segment,
        )?;
        Ok(waiter)
    }

    fn write_page_async(&self, idx: usize, frame: &Frame) -> Result<BioWaiter> {
        let inner = self.inner.read();
        let sector_size = inner.fs().sector_size();

        let sector_id = inner.get_sector_id(idx * PAGE_SIZE / inner.fs().sector_size())?;

        // FIXME: We may need to truncate the file if write_page fails.
        // To fix this issue, we need to change the interface of the PageCacheBackend trait.
        let bio_segment =
            BioSegment::new_from_segment(frame.clone().into(), BioDirection::ToDevice);
        let waiter = inner.fs().block_device().write_blocks_async(
            BlockId::from_offset(sector_id * inner.fs().sector_size()),
            bio_segment,
        )?;
        Ok(waiter)
    }

    fn npages(&self) -> usize {
        self.inner.read().size.align_up(PAGE_SIZE) / PAGE_SIZE
    }
}

impl ExfatInodeInner {
    /// The hash_value to index inode. This should be unique in the whole fs.
    /// Currently use dentry set physical position as hash value except for root(0).
    fn hash_index(&self) -> usize {
        if self.ino == EXFAT_ROOT_INO {
            return ROOT_INODE_HASH;
        }

        make_hash_index(
            self.dentry_set_position.0.cluster_id(),
            self.dentry_set_position.1 as u32,
        )
    }

    fn get_parent_inode(&self) -> Option<Arc<ExfatInode>> {
        //FIXME: What if parent inode is evicted? How can I find it?
        self.fs().find_opened_inode(self.parent_hash)
    }

    /// Get physical sector id from logical sector id for this Inode.
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

    /// Get the physical cluster id from the logical cluster id in the inode.
    fn get_physical_cluster(&self, logical: ClusterID) -> Result<ClusterID> {
        let chain = self.start_chain.walk(logical)?;
        Ok(chain.cluster_id())
    }

    /// The number of clusters allocated.
    fn num_clusters(&self) -> u32 {
        self.start_chain.num_clusters()
    }

    fn is_sync(&self) -> bool {
        false
    }

    fn fs(&self) -> Arc<ExfatFS> {
        self.fs.upgrade().unwrap()
    }

    /// Only valid for directory, check if the dir is empty.
    fn is_empty_dir(&self) -> Result<bool> {
        if !self.inode_type.is_directory() {
            return_errno!(Errno::ENOTDIR)
        }
        Ok(self.num_sub_inodes == 0)
    }

    fn make_mode(&self) -> InodeMode {
        self.attr
            .make_mode(self.fs().mount_option(), InodeMode::all())
    }

    fn count_num_sub_inode_and_dir(&self, fs_guard: &MutexGuard<()>) -> Result<(usize, usize)> {
        if !self.start_chain.is_current_cluster_valid() {
            return Ok((0, 0));
        }

        let iterator = ExfatDentryIterator::new(self.page_cache.pages().dup(), 0, Some(self.size))?;
        let mut sub_inodes = 0;
        let mut sub_dirs = 0;
        for dentry_result in iterator {
            let dentry = dentry_result?;
            if let ExfatDentry::File(file) = dentry {
                sub_inodes += 1;
                if FatAttr::from_bits_truncate(file.attribute).contains(FatAttr::DIRECTORY) {
                    sub_dirs += 1;
                }
            }
        }
        Ok((sub_inodes, sub_dirs))
    }

    /// Resize current inode to new_size.
    /// The `size_allocated` field in inode can be enlarged, while the `size` field will not.
    fn resize(&mut self, new_size: usize, fs_guard: &MutexGuard<()>) -> Result<()> {
        let fs = self.fs();
        let cluster_size = fs.cluster_size();

        let num_clusters = self.num_clusters();
        let new_num_clusters = (new_size.align_up(cluster_size) / cluster_size) as u32;

        let sync = self.is_sync();

        match new_num_clusters.cmp(&num_clusters) {
            Ordering::Greater => {
                // New clusters should be allocated.
                self.start_chain
                    .extend_clusters(new_num_clusters - num_clusters, sync)?;
            }
            Ordering::Less => {
                // Some exist clusters should be truncated.
                self.start_chain
                    .remove_clusters_from_tail(num_clusters - new_num_clusters, sync)?;
                if new_size < self.size {
                    // Valid data is truncated.
                    self.size = new_size;
                }
            }
            _ => {}
        };
        self.size_allocated = new_size;

        Ok(())
    }

    /// Update inode information back to the disk to sync this inode.
    /// Should lock the file system before calling this function.
    fn write_inode(&self, sync: bool, fs_guard: &MutexGuard<()>) -> Result<()> {
        // Root dir should not be updated.
        if self.ino == EXFAT_ROOT_INO {
            return Ok(());
        }

        // If the inode or its parent is already unlinked, there is no need for updating it.
        if self.is_deleted || !self.dentry_set_position.0.is_current_cluster_valid() {
            return Ok(());
        }

        let parent = self.get_parent_inode().unwrap_or_else(|| unimplemented!());
        let page_cache = parent.page_cache().unwrap();

        // Need to read the latest dentry set from parent inode.

        let mut dentry_set =
            ExfatDentrySet::read_from(page_cache.dup(), self.dentry_entry as usize * DENTRY_SIZE)?;

        let mut file_dentry = dentry_set.get_file_dentry();
        let mut stream_dentry = dentry_set.get_stream_dentry();

        file_dentry.attribute = self.attr.bits();

        file_dentry.create_utc_offset = self.ctime.utc_offset;
        file_dentry.create_date = self.ctime.date;
        file_dentry.create_time = self.ctime.time;
        file_dentry.create_time_cs = self.ctime.increment_10ms;

        file_dentry.modify_utc_offset = self.mtime.utc_offset;
        file_dentry.modify_date = self.mtime.date;
        file_dentry.modify_time = self.mtime.time;
        file_dentry.modify_time_cs = self.mtime.increment_10ms;

        file_dentry.access_utc_offset = self.atime.utc_offset;
        file_dentry.access_date = self.atime.date;
        file_dentry.access_time = self.atime.time;

        stream_dentry.valid_size = self.size as u64;
        stream_dentry.size = self.size_allocated as u64;
        stream_dentry.start_cluster = self.start_chain.cluster_id();
        stream_dentry.flags = self.start_chain.flags().bits();

        dentry_set.set_file_dentry(&file_dentry);
        dentry_set.set_stream_dentry(&stream_dentry);
        dentry_set.update_checksum();

        //Update the page cache of parent inode.
        let start_off = self.dentry_entry as usize * DENTRY_SIZE;
        let bytes = dentry_set.to_le_bytes();

        page_cache.write_bytes(start_off, &bytes)?;
        if sync {
            page_cache.decommit(start_off..start_off + bytes.len())?;
        }

        Ok(())
    }

    /// Read all sub-inodes from the given position(offset) in this directory.
    /// The number of inodes to read is given by dir_cnt.
    /// Return (the new offset after read, the number of sub-inodes read).
    fn visit_sub_inodes(
        &self,
        offset: usize,
        dir_cnt: usize,
        visitor: &mut dyn DirentVisitor,
        fs_guard: &MutexGuard<()>,
    ) -> Result<(usize, usize)> {
        if !self.inode_type.is_directory() {
            return_errno!(Errno::ENOTDIR)
        }
        if dir_cnt == 0 {
            return Ok((offset, 0));
        }

        let fs = self.fs();
        let cluster_size = fs.cluster_size();

        let mut iter = ExfatDentryIterator::new(self.page_cache.pages().dup(), offset, None)?;

        let mut dir_read = 0;
        let mut current_off = offset;

        // We need to skip empty or deleted dentry.
        while dir_read < dir_cnt {
            let dentry_result = iter.next();

            if dentry_result.is_none() {
                return_errno_with_message!(Errno::ENOENT, "inode data not available")
            }

            let dentry = dentry_result.unwrap()?;

            if let ExfatDentry::File(file) = dentry {
                if let Ok(dentry_set_size) =
                    self.visit_sub_inode(&file, &mut iter, current_off, visitor, fs_guard)
                {
                    current_off += dentry_set_size;
                    dir_read += 1;
                } else {
                    return Ok((current_off, dir_read));
                }
            } else {
                current_off += DENTRY_SIZE;
            }
        }

        Ok((current_off, dir_read))
    }

    /// Visit a sub-inode at offset. Return the dentry-set size of the sub-inode.
    /// Dirent visitor will extract information from the inode.
    fn visit_sub_inode(
        &self,
        file_dentry: &ExfatFileDentry,
        iter: &mut ExfatDentryIterator,
        offset: usize,
        visitor: &mut dyn DirentVisitor,
        fs_guard: &MutexGuard<()>,
    ) -> Result<usize> {
        if !self.inode_type.is_directory() {
            return_errno!(Errno::ENOTDIR)
        }
        let fs = self.fs();
        let cluster_size = fs.cluster_size();

        let dentry_position = self.start_chain.walk_to_cluster_at_offset(offset)?;

        if let Some(child_inode) = fs.find_opened_inode(make_hash_index(
            dentry_position.0.cluster_id(),
            dentry_position.1 as u32,
        )) {
            // Inode already exists.
            let child_inner = child_inode.inner.read();

            for i in 0..(child_inner.dentry_set_size / DENTRY_SIZE - 1) {
                let dentry_result = iter.next();
                if dentry_result.is_none() {
                    return_errno_with_message!(Errno::ENOENT, "inode data not available")
                }
                dentry_result.unwrap()?;
            }
            visitor.visit(
                &child_inner.name.to_string(),
                child_inner.ino,
                child_inner.inode_type,
                offset,
            )?;

            Ok(child_inner.dentry_set_size)
        } else {
            // Otherwise, create a new node and insert it to hash map.
            let ino = fs.alloc_inode_number();
            let child_inode = ExfatInode::read_from_iterator(
                fs.clone(),
                offset / DENTRY_SIZE,
                dentry_position,
                file_dentry,
                iter,
                self.hash_index(),
                fs_guard,
            )?;
            let _ = fs.insert_inode(child_inode.clone());
            let child_inner = child_inode.inner.read();

            visitor.visit(
                &child_inner.name.to_string(),
                ino,
                child_inner.inode_type,
                offset,
            )?;
            Ok(child_inner.dentry_set_size)
        }
    }

    /// Look up a target with "name", cur inode represent a dir.
    /// Return (target inode, dentries start offset, dentry set size).
    /// No inode should hold the write lock.
    fn lookup_by_name(
        &self,
        target_name: &str,
        case_sensitive: bool,
        fs_guard: &MutexGuard<()>,
    ) -> Result<Arc<ExfatInode>> {
        let fs = self.fs();

        let target_upcase = if !case_sensitive {
            fs.upcase_table().lock().str_to_upcase(target_name)?
        } else {
            target_name.to_string()
        };

        // FIXME: This isn't expected by the compiler.
        #[allow(non_local_definitions)]
        impl DirentVisitor for Vec<(String, usize)> {
            fn visit(
                &mut self,
                name: &str,
                ino: u64,
                type_: InodeType,
                offset: usize,
            ) -> Result<()> {
                self.push((name.into(), offset));
                Ok(())
            }
        }

        let mut name_and_offsets: Vec<(String, usize)> = vec![];
        self.visit_sub_inodes(
            0,
            self.num_sub_inodes as usize,
            &mut name_and_offsets,
            fs_guard,
        )?;

        for (name, offset) in name_and_offsets {
            let name_upcase = if !case_sensitive {
                fs.upcase_table().lock().str_to_upcase(&name)?
            } else {
                name
            };

            if name_upcase.eq(&target_upcase) {
                let chain_off = self.start_chain.walk_to_cluster_at_offset(offset)?;
                let hash = make_hash_index(chain_off.0.cluster_id(), chain_off.1 as u32);
                let inode = fs.find_opened_inode(hash).unwrap();

                return Ok(inode.clone());
            }
        }
        return_errno!(Errno::ENOENT)
    }

    fn delete_associated_secondary_clusters(
        &mut self,
        dentry: &ExfatDentry,
        fs_guard: &MutexGuard<()>,
    ) -> Result<()> {
        let fs = self.fs();
        let cluster_size = fs.cluster_size();
        match dentry {
            ExfatDentry::VendorAlloc(inner) => {
                if !fs.is_valid_cluster(inner.start_cluster) {
                    return Ok(());
                }
                let num_to_free = (inner.size as usize / cluster_size) as u32;
                ExfatChain::new(
                    self.fs.clone(),
                    inner.start_cluster,
                    Some(num_to_free),
                    FatChainFlags::ALLOC_POSSIBLE,
                )?
                .remove_clusters_from_tail(num_to_free, self.is_sync())?;
            }
            ExfatDentry::GenericSecondary(inner) => {
                if !fs.is_valid_cluster(inner.start_cluster) {
                    return Ok(());
                }
                let num_to_free = (inner.size as usize / cluster_size) as u32;
                ExfatChain::new(
                    self.fs.clone(),
                    inner.start_cluster,
                    Some(num_to_free),
                    FatChainFlags::ALLOC_POSSIBLE,
                )?
                .remove_clusters_from_tail(num_to_free, self.is_sync())?;
            }
            _ => {}
        };
        Ok(())
    }

    fn free_all_clusters(&mut self, fs_guard: &MutexGuard<()>) -> Result<()> {
        let num_clusters = self.num_clusters();
        self.start_chain
            .remove_clusters_from_tail(num_clusters, self.is_sync())
    }

    fn sync_metadata(&self, fs_guard: &MutexGuard<()>) -> Result<()> {
        self.fs().bitmap().lock().sync()?;
        self.write_inode(true, fs_guard)?;
        Ok(())
    }

    fn sync_data(&self, fs_guard: &MutexGuard<()>) -> Result<()> {
        self.page_cache.evict_range(0..self.size)?;
        Ok(())
    }

    fn sync_all(&self, fs_guard: &MutexGuard<()>) -> Result<()> {
        self.sync_metadata(fs_guard)?;
        self.sync_data(fs_guard)?;
        Ok(())
    }

    /// Update the metadata for current directory after a delete.
    /// Set is_dir if the deleted file is a directory.
    fn update_metadata_for_delete(&mut self, is_dir: bool) {
        self.num_sub_inodes -= 1;
        if is_dir {
            self.num_sub_dirs -= 1;
        }
    }

    fn update_atime(&mut self) -> Result<()> {
        self.atime = DosTimestamp::now()?;
        Ok(())
    }

    fn update_atime_and_mtime(&mut self) -> Result<()> {
        let now = DosTimestamp::now()?;
        self.atime = now;
        self.mtime = now;
        Ok(())
    }
}

impl ExfatInode {
    // TODO: Should be called when inode is evicted from fs.
    pub(super) fn reclaim_space(&self) -> Result<()> {
        let inner = self.inner.write();
        let fs = inner.fs();
        let fs_guard = fs.lock();
        self.inner.write().resize(0, &fs_guard)?;
        self.inner.read().page_cache.resize(0)?;
        Ok(())
    }

    pub(super) fn hash_index(&self) -> usize {
        self.inner.read().hash_index()
    }

    pub(super) fn is_deleted(&self) -> bool {
        self.inner.read().is_deleted
    }

    pub(super) fn build_root_inode(
        fs_weak: Weak<ExfatFS>,
        root_chain: ExfatChain,
    ) -> Result<Arc<ExfatInode>> {
        let sb = fs_weak.upgrade().unwrap().super_block();

        let root_cluster = sb.root_dir;

        let dentry_set_size = 0;

        let attr = FatAttr::DIRECTORY;

        let inode_type = InodeType::Dir;

        let ctime = DosTimestamp::now()?;

        let size = root_chain.num_clusters() as usize * sb.cluster_size as usize;

        let name = ExfatName::new();

        let inode = Arc::new_cyclic(|weak_self| ExfatInode {
            inner: RwMutex::new(ExfatInodeInner {
                ino: EXFAT_ROOT_INO,
                dentry_set_position: ExfatChainPosition::default(),
                dentry_set_size: 0,
                dentry_entry: 0,
                inode_type,
                attr,
                start_chain: root_chain,
                size,
                size_allocated: size,
                atime: ctime,
                mtime: ctime,
                ctime,
                num_sub_inodes: 0,
                num_sub_dirs: 0,
                name,
                is_deleted: false,
                parent_hash: 0,
                fs: fs_weak,
                page_cache: PageCache::with_capacity(size, weak_self.clone() as _).unwrap(),
            }),
            extension: Extension::new(),
        });

        let inner = inode.inner.upread();
        let fs = inner.fs();
        let fs_guard = fs.lock();

        let num_sub_inode_dir: (usize, usize) = inner.count_num_sub_inode_and_dir(&fs_guard)?;

        let mut inode_inner = inner.upgrade();

        inode_inner.num_sub_inodes = num_sub_inode_dir.0 as u32;
        inode_inner.num_sub_dirs = num_sub_inode_dir.1 as u32;

        Ok(inode.clone())
    }

    fn build_from_dentry_set(
        fs: Arc<ExfatFS>,
        dentry_set: &ExfatDentrySet,
        dentry_set_position: ExfatChainPosition,
        dentry_entry: u32,
        parent_hash: usize,
        fs_guard: &MutexGuard<()>,
    ) -> Result<Arc<ExfatInode>> {
        const EXFAT_MINIMUM_DENTRY: usize = 3;

        let ino = fs.alloc_inode_number();

        if dentry_set.len() < EXFAT_MINIMUM_DENTRY {
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
        let num_clusters = size_allocated.align_up(fs.cluster_size()) / fs.cluster_size();

        let start_chain = ExfatChain::new(
            fs_weak.clone(),
            start_cluster,
            Some(num_clusters as u32),
            chain_flag,
        )?;

        let name = dentry_set.get_name(fs.upcase_table())?;
        let inode = Arc::new_cyclic(|weak_self| ExfatInode {
            inner: RwMutex::new(ExfatInodeInner {
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
                num_sub_inodes: 0,
                num_sub_dirs: 0,
                name,
                is_deleted: false,
                parent_hash,
                fs: fs_weak,
                page_cache: PageCache::with_capacity(size, weak_self.clone() as _).unwrap(),
            }),
            extension: Extension::new(),
        });

        if matches!(inode_type, InodeType::Dir) {
            let inner = inode.inner.upread();
            let num_sub_inode_dir: (usize, usize) = inner.count_num_sub_inode_and_dir(fs_guard)?;

            let mut inode_inner = inner.upgrade();

            inode_inner.num_sub_inodes = num_sub_inode_dir.0 as u32;
            inode_inner.num_sub_dirs = num_sub_inode_dir.1 as u32;
        }

        Ok(inode)
    }

    /// The caller of the function should give a unique ino to assign to the inode.
    pub(super) fn read_from_iterator(
        fs: Arc<ExfatFS>,
        dentry_entry: usize,
        chain_pos: ExfatChainPosition,
        file_dentry: &ExfatFileDentry,
        iter: &mut ExfatDentryIterator,
        parent_hash: usize,
        fs_guard: &MutexGuard<()>,
    ) -> Result<Arc<Self>> {
        let dentry_set = ExfatDentrySet::read_from_iterator(file_dentry, iter)?;
        Self::build_from_dentry_set(
            fs,
            &dentry_set,
            chain_pos,
            dentry_entry as u32,
            parent_hash,
            fs_guard,
        )
    }

    /// Find empty dentry. If not found, expand the cluster chain.
    fn find_empty_dentries(&self, num_dentries: usize, fs_guard: &MutexGuard<()>) -> Result<usize> {
        let inner = self.inner.upread();

        let dentry_iterator =
            ExfatDentryIterator::new(inner.page_cache.pages().dup(), 0, Some(inner.size))?;

        let mut contiguous_unused = 0;
        let mut entry_id = 0;

        for dentry_result in dentry_iterator {
            let dentry = dentry_result?;
            match dentry {
                ExfatDentry::UnUsed | ExfatDentry::Deleted(_) => {
                    contiguous_unused += 1;
                }
                _ => {
                    contiguous_unused = 0;
                }
            }
            if contiguous_unused >= num_dentries {
                return Ok(entry_id - (num_dentries - 1));
            }
            entry_id += 1;
        }

        // Empty entries not found, allocate new cluster.

        if inner.size >= EXFAT_MAX_DENTRIES as usize * DENTRY_SIZE {
            return_errno!(Errno::ENOSPC)
        }
        let fs = inner.fs();
        let cluster_size = fs.cluster_size();
        let cluster_to_be_allocated =
            (num_dentries * DENTRY_SIZE).align_up(cluster_size) / cluster_size;

        let is_sync = inner.is_sync();
        let old_size_allocated = inner.size_allocated;
        let new_size_allocated = old_size_allocated + cluster_size * cluster_to_be_allocated;
        {
            let mut inner = inner.upgrade();
            inner
                .start_chain
                .extend_clusters(cluster_to_be_allocated as u32, is_sync)?;

            inner.size_allocated = new_size_allocated;
            inner.size = new_size_allocated;

            inner.page_cache.resize(new_size_allocated)?;
        }
        let inner = self.inner.read();

        // We need to write unused dentries (i.e. 0) to page cache.
        inner
            .page_cache
            .pages()
            .clear(old_size_allocated..new_size_allocated)?;

        Ok(entry_id)
    }

    /// Add new dentries. Create a new file or folder.
    fn add_entry(
        &self,
        name: &str,
        inode_type: InodeType,
        mode: InodeMode,
        fs_guard: &MutexGuard<()>,
    ) -> Result<Arc<ExfatInode>> {
        if name.len() > MAX_NAME_LENGTH {
            return_errno!(Errno::ENAMETOOLONG)
        }
        let fs = self.inner.read().fs();

        // TODO: remove trailing periods of pathname.
        // Do not allow creation of files with names ending with period(s).

        let name_dentries = name.len().div_ceil(EXFAT_FILE_NAME_LEN);
        let num_dentries = name_dentries + 2; // FILE Entry + Stream Entry + Name Entry

        // We update the size of inode before writing page_cache, but it is fine since we've cleaned the page_cache.
        let entry = self.find_empty_dentries(num_dentries, fs_guard)? as u32;

        let dentry_set = ExfatDentrySet::from(fs.clone(), name, inode_type, mode)?;

        let start_off = entry as usize * DENTRY_SIZE;
        let end_off = (entry as usize + num_dentries) * DENTRY_SIZE;

        let inner = self.inner.upread();
        inner
            .page_cache
            .pages()
            .write_bytes(start_off, &dentry_set.to_le_bytes())?;

        let mut inner = inner.upgrade();

        inner.num_sub_inodes += 1;
        if inode_type.is_directory() {
            inner.num_sub_dirs += 1;
        }

        let pos = inner.start_chain.walk_to_cluster_at_offset(start_off)?;

        let new_inode = ExfatInode::build_from_dentry_set(
            fs.clone(),
            &dentry_set,
            pos,
            entry,
            inner.hash_index(),
            fs_guard,
        )?;

        if inode_type.is_directory() && !fs.mount_option().zero_size_dir {
            // TODO: We need to resize the directory so that it contains at least 1 cluster if zero_size_dir is not enabled.
            // new_inode.resize(new_size)
        }
        Ok(new_inode)
    }

    // Delete dentry set for current directory.
    fn delete_dentry_set(
        &self,
        offset: usize,
        len: usize,
        fs_guard: &MutexGuard<()>,
    ) -> Result<()> {
        let fs = self.inner.read().fs();
        let mut buf = vec![0; len];

        self.inner
            .read()
            .page_cache
            .pages()
            .read_bytes(offset, &mut buf)?;

        let num_dentry = len / DENTRY_SIZE;

        let cluster_size = fs.cluster_size();
        for i in 0..num_dentry {
            let buf_offset = DENTRY_SIZE * i;
            // Delete cluster chain if needed.
            let dentry = ExfatDentry::try_from(RawExfatDentry::from_bytes(
                &buf[buf_offset..buf_offset + DENTRY_SIZE],
            ))?;
            self.inner
                .write()
                .delete_associated_secondary_clusters(&dentry, fs_guard)?;
            // Mark this dentry as deleted.
            buf[buf_offset] &= 0x7F;
        }

        self.inner
            .read()
            .page_cache
            .pages()
            .write_bytes(offset, &buf)?;

        // FIXME: We must make sure that there are no spare tailing clusters in a directory.
        Ok(())
    }

    /// Copy metadata from the given inode.
    /// There will be no deadlock since this function is only used in rename and the arg "inode".
    /// is a temporary inode which is only accessible to current thread.
    fn copy_metadata_from(&self, inode: Arc<ExfatInode>) {
        let mut self_inner = self.inner.write();
        let other_inner = inode.inner.read();

        self_inner.dentry_set_position = other_inner.dentry_set_position.clone();
        self_inner.dentry_set_size = other_inner.dentry_set_size;
        self_inner.dentry_entry = other_inner.dentry_entry;
        self_inner.atime = other_inner.atime;
        self_inner.ctime = other_inner.ctime;
        self_inner.mtime = other_inner.mtime;
        self_inner.name = other_inner.name.clone();
        self_inner.is_deleted = other_inner.is_deleted;
        self_inner.parent_hash = other_inner.parent_hash;
    }

    fn update_subdir_parent_hash(&self, fs_guard: &MutexGuard<()>) -> Result<()> {
        let inner = self.inner.read();
        if !inner.inode_type.is_directory() {
            return Ok(());
        }
        let new_parent_hash = self.hash_index();
        let sub_dir = inner.num_sub_inodes;
        let mut child_offsets: Vec<usize> = vec![];
        // FIXME: This isn't expected by the compiler.
        #[allow(non_local_definitions)]
        impl DirentVisitor for Vec<usize> {
            fn visit(
                &mut self,
                name: &str,
                ino: u64,
                type_: InodeType,
                offset: usize,
            ) -> Result<()> {
                self.push(offset);
                Ok(())
            }
        }
        inner.visit_sub_inodes(0, sub_dir as usize, &mut child_offsets, fs_guard)?;

        let start_chain = inner.start_chain.clone();
        for offset in child_offsets {
            let child_dentry_pos = start_chain.walk_to_cluster_at_offset(offset)?;
            let child_hash =
                make_hash_index(child_dentry_pos.0.cluster_id(), child_dentry_pos.1 as u32);
            let child_inode = inner.fs().find_opened_inode(child_hash).unwrap();
            child_inode.inner.write().parent_hash = new_parent_hash;
        }
        Ok(())
    }

    /// Unlink a file or remove a directory.
    /// Need to delete dentry set and inode.
    /// Delete the file contents if delete_content is set.
    fn delete_inode(
        &self,
        inode: Arc<ExfatInode>,
        delete_contents: bool,
        fs_guard: &MutexGuard<()>,
    ) -> Result<()> {
        // Delete directory contents directly.
        let is_dir = inode.inner.read().inode_type.is_directory();
        if delete_contents {
            if is_dir {
                inode.inner.write().resize(0, fs_guard)?;
                inode.inner.read().page_cache.resize(0)?;
            }
            // Set the delete flag.
            inode.inner.write().is_deleted = true;
        }
        // Remove the inode.
        self.inner.read().fs().remove_inode(inode.hash_index());
        // Remove dentry set.
        let dentry_set_offset = inode.inner.read().dentry_entry as usize * DENTRY_SIZE;
        let dentry_set_len = inode.inner.read().dentry_set_size;
        self.delete_dentry_set(dentry_set_offset, dentry_set_len, fs_guard)?;
        self.inner.write().update_metadata_for_delete(is_dir);
        Ok(())
    }
}

struct EmptyVistor;
impl DirentVisitor for EmptyVistor {
    fn visit(&mut self, name: &str, ino: u64, type_: InodeType, offset: usize) -> Result<()> {
        Ok(())
    }
}
fn is_block_aligned(off: usize) -> bool {
    off % PAGE_SIZE == 0
}

fn check_corner_cases_for_rename(
    old_inode: &Arc<ExfatInode>,
    exist_inode: &Arc<ExfatInode>,
) -> Result<()> {
    // Check for two corner cases here.
    let old_inode_is_dir = old_inode.inner.read().inode_type.is_directory();
    // If old_inode represents a directory, the exist 'new_name' must represents a empty directory.
    if old_inode_is_dir && !exist_inode.inner.read().is_empty_dir()? {
        return_errno!(Errno::ENOTEMPTY)
    }
    // If old_inode represents a file, the exist 'new_name' must also represents a file.
    if !old_inode_is_dir && exist_inode.inner.read().inode_type.is_directory() {
        return_errno!(Errno::EISDIR)
    }
    Ok(())
}

impl Inode for ExfatInode {
    fn ino(&self) -> u64 {
        self.inner.read().ino
    }

    fn size(&self) -> usize {
        self.inner.read().size
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        let inner = self.inner.upread();

        if inner.inode_type.is_directory() {
            return_errno!(Errno::EISDIR)
        }

        let file_size = inner.size;
        let fs = inner.fs();
        let fs_guard = fs.lock();

        inner.upgrade().resize(new_size, &fs_guard)?;

        // Update the size of page cache.
        let inner = self.inner.read();

        // We will delay updating the page_cache size when enlarging an inode until the real write.
        if new_size < file_size {
            self.inner.read().page_cache.resize(new_size)?;
        }

        // Sync this inode since size has changed.
        if inner.is_sync() {
            inner.sync_metadata(&fs_guard)?;
        }

        Ok(())
    }

    fn metadata(&self) -> crate::fs::utils::Metadata {
        let inner = self.inner.read();

        let blk_size = inner.fs().super_block().sector_size as usize;

        let nlinks = if inner.inode_type.is_directory() {
            (inner.num_sub_dirs + 2) as usize
        } else {
            1
        };

        Metadata {
            dev: 0,
            ino: inner.ino,
            size: inner.size,
            blk_size,
            blocks: inner.size.div_ceil(blk_size),
            atime: inner.atime.as_duration().unwrap_or_default(),
            mtime: inner.mtime.as_duration().unwrap_or_default(),
            ctime: inner.ctime.as_duration().unwrap_or_default(),
            type_: inner.inode_type,
            mode: inner.make_mode(),
            nlinks,
            uid: Uid::new(inner.fs().mount_option().fs_uid as u32),
            gid: Gid::new(inner.fs().mount_option().fs_gid as u32),
            //real device
            rdev: 0,
        }
    }

    fn type_(&self) -> InodeType {
        self.inner.read().inode_type
    }

    fn mode(&self) -> Result<InodeMode> {
        Ok(self.inner.read().make_mode())
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        //Pass through
        Ok(())
    }

    fn atime(&self) -> Duration {
        self.inner.read().atime.as_duration().unwrap_or_default()
    }

    fn set_atime(&self, time: Duration) {
        self.inner.write().atime = DosTimestamp::from_duration(time).unwrap_or_default();
    }

    fn mtime(&self) -> Duration {
        self.inner.read().mtime.as_duration().unwrap_or_default()
    }

    fn set_mtime(&self, time: Duration) {
        self.inner.write().mtime = DosTimestamp::from_duration(time).unwrap_or_default();
    }

    fn ctime(&self) -> Duration {
        self.inner.read().ctime.as_duration().unwrap_or_default()
    }

    fn set_ctime(&self, time: Duration) {
        self.inner.write().ctime = DosTimestamp::from_duration(time).unwrap_or_default();
    }

    fn owner(&self) -> Result<Uid> {
        Ok(Uid::new(
            self.inner.read().fs().mount_option().fs_uid as u32,
        ))
    }

    fn set_owner(&self, uid: Uid) -> Result<()> {
        // Pass through.
        Ok(())
    }

    fn group(&self) -> Result<Gid> {
        Ok(Gid::new(
            self.inner.read().fs().mount_option().fs_gid as u32,
        ))
    }

    fn set_group(&self, gid: Gid) -> Result<()> {
        // Pass through.
        Ok(())
    }

    fn fs(&self) -> alloc::sync::Arc<dyn crate::fs::utils::FileSystem> {
        self.inner.read().fs()
    }

    fn page_cache(&self) -> Option<Vmo<Full>> {
        Some(self.inner.read().page_cache.pages().dup())
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let inner = self.inner.upread();
        if inner.inode_type.is_directory() {
            return_errno!(Errno::EISDIR)
        }
        let (read_off, read_len) = {
            let file_size = inner.size;
            let start = file_size.min(offset);
            let end = file_size.min(offset + writer.avail());
            (start, end - start)
        };
        inner.page_cache.pages().read(read_off, writer)?;

        inner.upgrade().update_atime()?;
        Ok(read_len)
    }

    // The offset and the length of buffer must be multiples of the block size.
    fn read_direct_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let inner = self.inner.upread();
        if inner.inode_type.is_directory() {
            return_errno!(Errno::EISDIR)
        }
        if !is_block_aligned(offset) || !is_block_aligned(writer.avail()) {
            return_errno_with_message!(Errno::EINVAL, "not block-aligned");
        }

        let sector_size = inner.fs().sector_size();

        let (read_off, read_len) = {
            let file_size = inner.size;
            let start = file_size.min(offset).align_down(sector_size);
            let end = file_size
                .min(offset + writer.avail())
                .align_down(sector_size);
            (start, end - start)
        };

        inner
            .page_cache
            .discard_range(read_off..read_off + read_len);

        let mut buf_offset = 0;
        let bio_segment = BioSegment::alloc(1, BioDirection::FromDevice);

        let start_pos = inner.start_chain.walk_to_cluster_at_offset(read_off)?;
        let cluster_size = inner.fs().cluster_size();
        let mut cur_cluster = start_pos.0.clone();
        let mut cur_offset = start_pos.1;
        for _ in Bid::from_offset(read_off)..Bid::from_offset(read_off + read_len) {
            let physical_bid =
                Bid::from_offset(cur_cluster.cluster_id() as usize * cluster_size + cur_offset);
            inner
                .fs()
                .block_device()
                .read_blocks(physical_bid, bio_segment.clone())?;
            bio_segment.reader().unwrap().read_fallible(writer)?;
            buf_offset += BLOCK_SIZE;

            cur_offset += BLOCK_SIZE;
            if cur_offset >= cluster_size {
                cur_cluster = cur_cluster.walk(1)?;
                cur_offset %= BLOCK_SIZE;
            }
        }

        inner.upgrade().update_atime()?;
        Ok(read_len)
    }

    fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        let write_len = reader.remain();
        // We need to obtain the fs lock to resize the file.
        let new_size = {
            let mut inner = self.inner.write();
            if inner.inode_type.is_directory() {
                return_errno!(Errno::EISDIR)
            }

            let file_size = inner.size;
            let file_allocated_size = inner.size_allocated;
            let new_size = offset + write_len;
            let fs = inner.fs();
            let fs_guard = fs.lock();
            if new_size > file_size {
                if new_size > file_allocated_size {
                    inner.resize(new_size, &fs_guard)?;
                }
                inner.page_cache.resize(new_size)?;
            }
            new_size.max(file_size)
        };

        // Locks released here, so that file write can be parallelized.
        let inner = self.inner.upread();
        inner.page_cache.pages().write(offset, reader)?;

        // Update timestamps and size.
        {
            let mut inner = inner.upgrade();

            inner.update_atime_and_mtime()?;
            inner.size = new_size;
        }

        let inner = self.inner.read();

        // Write data back.
        if inner.is_sync() {
            let fs = inner.fs();
            let fs_guard = fs.lock();
            inner.sync_all(&fs_guard)?;
        }

        Ok(write_len)
    }

    fn write_direct_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        let write_len = reader.remain();
        let inner = self.inner.upread();
        if inner.inode_type.is_directory() {
            return_errno!(Errno::EISDIR)
        }
        if !is_block_aligned(offset) || !is_block_aligned(write_len) {
            return_errno_with_message!(Errno::EINVAL, "not block-aligned");
        }

        let file_size = inner.size;
        let file_allocated_size = inner.size_allocated;
        let end_offset = offset + write_len;

        let start = offset.min(file_size);
        let end = end_offset.min(file_size);
        inner.page_cache.discard_range(start..end);

        let new_size = {
            let mut inner = inner.upgrade();
            if end_offset > file_size {
                let fs = inner.fs();
                let fs_guard = fs.lock();
                if end_offset > file_allocated_size {
                    inner.resize(end_offset, &fs_guard)?;
                }
                inner.page_cache.resize(end_offset)?;
            }
            file_size.max(end_offset)
        };

        let inner = self.inner.upread();

        let bio_segment = BioSegment::alloc(1, BioDirection::ToDevice);
        let start_pos = inner.start_chain.walk_to_cluster_at_offset(offset)?;
        let cluster_size = inner.fs().cluster_size();
        let mut cur_cluster = start_pos.0.clone();
        let mut cur_offset = start_pos.1;
        for _ in Bid::from_offset(offset)..Bid::from_offset(end_offset) {
            bio_segment.writer().unwrap().write_fallible(reader)?;
            let physical_bid =
                Bid::from_offset(cur_cluster.cluster_id() as usize * cluster_size + cur_offset);
            let fs = inner.fs();
            fs.block_device()
                .write_blocks(physical_bid, bio_segment.clone())?;

            cur_offset += BLOCK_SIZE;
            if cur_offset >= cluster_size {
                cur_cluster = cur_cluster.walk(1)?;
                cur_offset %= BLOCK_SIZE;
            }
        }

        {
            let mut inner = inner.upgrade();
            inner.update_atime_and_mtime()?;
            inner.size = new_size;
        }

        let inner = self.inner.read();
        // Sync this inode since size has changed.
        if inner.is_sync() {
            let fs = inner.fs();
            let fs_guard = fs.lock();
            inner.sync_metadata(&fs_guard)?;
        }

        Ok(write_len)
    }

    fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>> {
        let fs = self.inner.read().fs();
        let fs_guard = fs.lock();
        {
            let inner = self.inner.read();
            if !inner.inode_type.is_directory() {
                return_errno!(Errno::ENOTDIR)
            }
            if name.len() > MAX_NAME_LENGTH {
                return_errno!(Errno::ENAMETOOLONG)
            }

            if inner.lookup_by_name(name, false, &fs_guard).is_ok() {
                return_errno!(Errno::EEXIST)
            }
        }

        let result = self.add_entry(name, type_, mode, &fs_guard)?;
        let _ = fs.insert_inode(result.clone());

        self.inner.write().update_atime_and_mtime()?;

        let inner = self.inner.read();

        if inner.is_sync() {
            inner.sync_all(&fs_guard)?;
        }

        Ok(result)
    }

    fn mknod(&self, name: &str, mode: InodeMode, type_: MknodType) -> Result<Arc<dyn Inode>> {
        return_errno_with_message!(Errno::EINVAL, "unsupported operation")
    }

    fn readdir_at(&self, dir_cnt: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        let inner = self.inner.upread();

        if dir_cnt >= (inner.num_sub_inodes + 2) as usize {
            return Ok(0);
        }

        let mut empty_visitor = EmptyVistor;

        let dir_read = {
            let fs = inner.fs();
            let fs_guard = fs.lock();

            let mut dir_read = 0usize;

            if dir_cnt == 0
                && visitor
                    .visit(".", inner.ino, inner.inode_type, 0xFFFFFFFFFFFFFFFEusize)
                    .is_ok()
            {
                dir_read += 1;
            }

            if dir_cnt <= 1 {
                let parent_inode = inner.get_parent_inode().unwrap();
                let parent_inner = parent_inode.inner.read();
                let ino = parent_inner.ino;
                let type_ = parent_inner.inode_type;
                if visitor
                    .visit("..", ino, type_, 0xFFFFFFFFFFFFFFFFusize)
                    .is_ok()
                {
                    dir_read += 1;
                }
            }

            // Skip . and ..
            let dir_to_skip = if dir_cnt >= 2 { dir_cnt - 2 } else { 0 };

            // Skip previous directories.
            let (off, _) = inner.visit_sub_inodes(0, dir_to_skip, &mut empty_visitor, &fs_guard)?;
            let (_, read) = inner.visit_sub_inodes(
                off,
                inner.num_sub_inodes as usize - dir_to_skip,
                visitor,
                &fs_guard,
            )?;
            dir_read += read;
            dir_read
        };

        inner.upgrade().update_atime()?;

        Ok(dir_read)
    }

    fn link(&self, old: &Arc<dyn Inode>, name: &str) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL, "unsupported operation")
    }

    fn unlink(&self, name: &str) -> Result<()> {
        if !self.inner.read().inode_type.is_directory() {
            return_errno!(Errno::ENOTDIR)
        }
        if name.len() > MAX_NAME_LENGTH {
            return_errno!(Errno::ENAMETOOLONG)
        }
        if name == "." || name == ".." {
            return_errno!(Errno::EISDIR)
        }

        let fs = self.inner.read().fs();
        let fs_guard = fs.lock();

        let inode = self.inner.read().lookup_by_name(name, true, &fs_guard)?;

        // FIXME: we need to step by following line to avoid deadlock.
        if inode.type_() != InodeType::File {
            return_errno!(Errno::EISDIR)
        }
        self.delete_inode(inode, true, &fs_guard)?;
        self.inner.write().update_atime_and_mtime()?;

        let inner = self.inner.read();
        if inner.is_sync() {
            inner.sync_all(&fs_guard)?;
        }

        Ok(())
    }

    fn rmdir(&self, name: &str) -> Result<()> {
        if !self.inner.read().inode_type.is_directory() {
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

        let fs = self.inner.read().fs();
        let fs_guard = fs.lock();

        let inode = self.inner.read().lookup_by_name(name, true, &fs_guard)?;

        if inode.inner.read().inode_type != InodeType::Dir {
            return_errno!(Errno::ENOTDIR)
        } else if !inode.inner.read().is_empty_dir()? {
            // Check if directory to be deleted is empty.
            return_errno!(Errno::ENOTEMPTY)
        }
        self.delete_inode(inode, true, &fs_guard)?;
        self.inner.write().update_atime_and_mtime()?;

        let inner = self.inner.read();
        // Sync this inode since size has changed.
        if inner.is_sync() {
            inner.sync_all(&fs_guard)?;
        }

        Ok(())
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        // FIXME: Readdir should be immutable instead of mutable, but there will be no performance issues due to the global fs lock.
        let inner = self.inner.upread();
        if !inner.inode_type.is_directory() {
            return_errno!(Errno::ENOTDIR)
        }

        if name.len() > MAX_NAME_LENGTH {
            return_errno!(Errno::ENAMETOOLONG)
        }

        let inode = {
            let fs = inner.fs();
            let fs_guard = fs.lock();
            inner.lookup_by_name(name, true, &fs_guard)?
        };

        inner.upgrade().update_atime()?;

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
        if !self.inner.read().inode_type.is_directory()
            || !target_.inner.read().inode_type.is_directory()
        {
            return_errno!(Errno::ENOTDIR)
        }

        let fs = self.inner.read().fs();
        let fs_guard = fs.lock();
        // Rename something to itself, return success directly.
        let up_old_name = fs.upcase_table().lock().str_to_upcase(old_name)?;
        let up_new_name = fs.upcase_table().lock().str_to_upcase(new_name)?;
        if self.inner.read().ino == target_.inner.read().ino && up_old_name.eq(&up_new_name) {
            return Ok(());
        }

        // Read 'old_name' file or dir and its dentries.
        let old_inode = self
            .inner
            .read()
            .lookup_by_name(old_name, true, &fs_guard)?;
        // FIXME: Users may be confused, since inode with the same upper case name will be removed.
        let lookup_exist_result = target_
            .inner
            .read()
            .lookup_by_name(new_name, false, &fs_guard);
        // Check for the corner cases.
        if let Ok(ref exist_inode) = lookup_exist_result {
            check_corner_cases_for_rename(&old_inode, exist_inode)?;
        }

        // All checks are done here. This is a valid rename and it needs to modify the metadata.
        self.delete_inode(old_inode.clone(), false, &fs_guard)?;
        // Create the new dentries.
        let new_inode =
            target_.add_entry(new_name, old_inode.type_(), old_inode.mode()?, &fs_guard)?;
        // Update metadata.
        old_inode.copy_metadata_from(new_inode);
        // Update its children's parent_hash.
        old_inode.update_subdir_parent_hash(&fs_guard)?;
        // Insert back.
        let _ = fs.insert_inode(old_inode.clone());
        // Remove the exist 'new_name' file.
        if let Ok(exist_inode) = lookup_exist_result {
            target_.delete_inode(exist_inode, true, &fs_guard)?;
        }
        // Update the times.
        self.inner.write().update_atime_and_mtime()?;
        target_.inner.write().update_atime_and_mtime()?;
        // Sync
        if self.inner.read().is_sync() || target_.inner.read().is_sync() {
            // TODO: what if fs crashed between syncing?
            old_inode.inner.read().sync_all(&fs_guard)?;
            target_.inner.read().sync_all(&fs_guard)?;
            self.inner.read().sync_all(&fs_guard)?;
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
        return_errno_with_message!(Errno::EINVAL, "unsupported operation")
    }

    fn sync_all(&self) -> Result<()> {
        let inner = self.inner.read();
        let fs = inner.fs();
        let fs_guard = fs.lock();
        inner.sync_all(&fs_guard)?;

        fs.block_device().sync()?;

        Ok(())
    }

    fn sync_data(&self) -> Result<()> {
        let inner = self.inner.read();
        let fs = inner.fs();
        let fs_guard = fs.lock();
        inner.sync_data(&fs_guard)?;

        fs.block_device().sync()?;

        Ok(())
    }

    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }

    fn is_dentry_cacheable(&self) -> bool {
        true
    }

    fn extension(&self) -> Option<&Extension> {
        Some(&self.extension)
    }
}
