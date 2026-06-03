// SPDX-License-Identifier: MPL-2.0

//! Core `Ext2` filesystem state for a mounted ext2 volume.
//!
//! This module owns `Ext2`, the top-level handle for a mounted ext2 volume.
//! It is the filesystem-wide coordination point for mount state, inode
//! routing, block allocation, and writeback. Group-local metadata and inode
//! caches remain owned by the corresponding `BlockGroup`.
//!
//! # Locking
//!
//! `Ext2` uses an `RwMutex` around the `SuperBlock` for filesystem-wide
//! state: read mode for queries, write mode for allocation, deallocation,
//! and sync. The `block_groups` vector itself is immutable after mount;
//! each `BlockGroup` carries its own internal locks (see `block_group`
//! module documentation). The `group_descriptors_segment` has no dedicated
//! lock — it is loaded once at mount and updated during `sync` under each
//! group's metadata write guard, with disjoint descriptor offsets ensuring
//! no two writers touch the same bytes. `next_generation` is an `AtomicU32`
//! incremented once per newly allocated inode.

use core::sync::atomic::{AtomicU32, Ordering};

use aster_block::bio::BioCompleteFn;
use device_id::DeviceId;

use super::{
    block_group::{BlockGroup, RawBlockGroup},
    inode::{FilePerm, Inode, InodeDesc, RawInode},
    prelude::*,
    super_block::{RawSuperBlock, SUPER_BLOCK_OFFSET, SuperBlock},
};
use crate::{
    fs::{ext2::utils, vfs::file_system::FsEventSubscriberStats},
    process::{Gid, UserNamespace, credentials::capabilities::CapSet, posix_thread::AsPosixThread},
    security::lsm::hooks as lsm_hooks,
    thread::Thread,
};

/// The root inode number defined by the ext2 on-disk format.
pub(super) const ROOT_INO: u32 = 2;

/// Top-level handle for a mounted ext2 filesystem.
///
/// Owns the superblock, block group array, and group descriptor table.
/// Filesystem-wide operations are routed through the block group selected by
/// the target inode or block address.
#[derive(Debug)]
pub struct Ext2 {
    /// Backing block device.
    block_device: Arc<dyn BlockDevice>,
    /// Superblock with dirty tracking.
    super_block: RwMutex<Dirty<SuperBlock>>,
    /// Block group descriptors and caches.
    block_groups: Vec<BlockGroup>,
    /// Inodes per group.
    nr_inodes_per_group: u32,
    /// Group descriptor table segment.
    group_descriptors_segment: USegment,
    /// Runtime mount options that affect block-count reporting.
    mount_options: Ext2MountOptions,
    /// FS event stats for VFS.
    fs_event_subscriber_stats: FsEventSubscriberStats,
    /// Per-filesystem inode generation counter.
    next_generation: AtomicU32,
    /// Weak self reference for inode back-pointers.
    self_ref: Weak<Ext2>,
}

/// Policy for how `statfs` reports the total block count.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum StatBlockAccounting {
    /// Excludes filesystem overhead (superblock, group descriptors, bitmaps,
    /// inode tables) from the reported total.
    ///
    /// This is the default and corresponds to Linux's `bsddf` mount option.
    #[default]
    ExcludeOverhead,
    /// Includes all blocks on the device in the reported total, regardless of
    /// whether they hold metadata.
    ///
    /// This corresponds to Linux's `minixdf` mount option.
    IncludeOverhead,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Ext2MountOptions {
    stat_block_accounting: StatBlockAccounting,
}

impl Ext2MountOptions {
    /// Parses ext2 mount options that control block-count reporting.
    fn parse(data: Option<&CStr>) -> Self {
        let mut options = Self::default();
        let Some(data) = data else {
            return options;
        };

        let data = data.to_string_lossy();
        for token in data.split(',') {
            match token.trim() {
                "bsddf" => options.stat_block_accounting = StatBlockAccounting::ExcludeOverhead,
                "minixdf" => options.stat_block_accounting = StatBlockAccounting::IncludeOverhead,
                _ => {}
            }
        }

        options
    }
}

impl Ext2 {
    /// Opens and loads an Ext2 filesystem from a block device.
    pub(super) fn open(device: Arc<dyn BlockDevice>, data: Option<&CStr>) -> Result<Arc<Self>> {
        let super_block = {
            let raw_super_block = device.read_val::<RawSuperBlock>(SUPER_BLOCK_OFFSET)?;
            SuperBlock::try_from(raw_super_block)?
        };
        let block_size = super_block.block_size();
        if block_size != BLOCK_SIZE {
            return_errno_with_message!(Errno::EINVAL, "currently only 4096-byte block size");
        }

        let mount_options = Ext2MountOptions::parse(data);

        let nr_inodes_per_group = super_block.nr_inodes_per_group();

        let group_descriptors_segment = {
            let nr_block_groups = super_block.nr_block_groups() as usize;
            let group_desc_bytes = nr_block_groups * size_of::<RawBlockGroup>();
            let nblocks = group_desc_bytes.div_ceil(BLOCK_SIZE);

            let segment = FrameAllocOptions::new()
                .zeroed(false)
                .alloc_segment(nblocks)?;
            let bio_segment =
                BioSegment::new_from_segment(segment.clone().into(), BioDirection::FromDevice);
            match device.read_blocks(
                Bid::new(super_block.group_descriptors_bid(0) as u64),
                bio_segment,
            )? {
                BioStatus::Complete => {}
                err_status => {
                    error!(
                        "Ext2: Failed to read group descriptor table: {:?}",
                        err_status
                    );
                    return Err(Error::from(err_status));
                }
            }
            let segment: USegment = segment.into();
            segment
        };

        let block_groups = {
            let nr_block_groups = super_block.nr_block_groups() as usize;
            let mut block_groups = Vec::with_capacity(nr_block_groups);
            for group_idx in 0..nr_block_groups {
                let group = BlockGroup::load(
                    &group_descriptors_segment,
                    group_idx,
                    &super_block,
                    device.clone(),
                )?;
                block_groups.push(group);
            }
            block_groups
        };

        let ext2 = Arc::new_cyclic(|weak_self| Ext2 {
            block_device: device,
            super_block: RwMutex::new(Dirty::new(super_block)),
            block_groups,
            nr_inodes_per_group,
            group_descriptors_segment,
            mount_options,
            fs_event_subscriber_stats: FsEventSubscriberStats::new(),
            next_generation: AtomicU32::new(utils::duration_to_ext2_secs(utils::now())),
            self_ref: weak_self.clone(),
        });

        Ok(ext2)
    }

    /// Returns the block device.
    pub(super) fn block_device(&self) -> &dyn BlockDevice {
        self.block_device.as_ref()
    }

    /// Returns the maximum regular file size supported by this ext2 instance.
    pub(super) fn max_file_size(&self) -> usize {
        self.super_block.read().max_file_size()
    }

    /// Returns whether Minix-style total blocks should be reported.
    pub(super) fn uses_minix_df(&self) -> bool {
        matches!(
            self.mount_options.stat_block_accounting,
            StatBlockAccounting::IncludeOverhead
        )
    }

    /// Returns a reference to the block group at `group_idx`.
    pub(super) fn block_group(&self, group_idx: usize) -> &BlockGroup {
        &self.block_groups[group_idx]
    }

    /// Returns a read guard of the superblock.
    pub(super) fn super_block(&self) -> RwMutexReadGuard<'_, Dirty<SuperBlock>> {
        self.super_block.read()
    }

    /// Returns the device ID of the underlying block device.
    pub(super) fn container_device_id(&self) -> DeviceId {
        self.block_device.id()
    }

    /// Returns the fs event subscriber stats.
    pub(super) fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        &self.fs_event_subscriber_stats
    }

    /// Returns the root inode.
    pub(super) fn root_inode(&self) -> Result<Arc<Inode>> {
        self.read_inode(ROOT_INO)
    }

    /// Reads an inode via per-block-group inode cache.
    pub(super) fn read_inode(&self, ino: Ext2Ino) -> Result<Arc<Inode>> {
        if ino == 0 {
            return_errno_with_message!(Errno::ENOENT, "inode 0 is not valid in ext2");
        }
        let group = self
            .find_group(ino)
            .ok_or_else(|| Error::with_message(Errno::EIO, "block group index out of range"))?;
        group.lookup_inode(ino, self.self_ref.clone())
    }

    /// Inserts a newly created inode into the corresponding block-group cache.
    pub(super) fn insert_inode(&self, inode: Arc<Inode>) {
        let ino = inode.ino();
        if ino == 0 {
            return;
        }
        if let Some(group) = self.find_group(ino) {
            group.insert_inode(inode);
        }
    }

    /// Removes one inode from the live block-group cache.
    pub(super) fn remove_inode(&self, ino: Ext2Ino) -> Option<Arc<Inode>> {
        if ino == 0 {
            return None;
        }
        self.find_group(ino)
            .and_then(|group| group.remove_inode(ino))
    }

    /// Writes an inode descriptor to the group's `PageCache`.
    pub(super) fn write_back_inode_desc(&self, ino: Ext2Ino, raw_inode: &RawInode) -> Result<()> {
        {
            let sb = self.super_block.read();
            // Apply ext2 inode-number validity rules before indexing groups.
            if (ino != ROOT_INO && ino < sb.first_ino()) || ino > sb.total_inodes() {
                return_errno_with_message!(Errno::EINVAL, "inode number out of valid range");
            }
        }

        let group = self
            .find_group(ino)
            .ok_or_else(|| Error::with_message(Errno::EIO, "block group index out of range"))?;

        group.write_back_inode_desc(ino, raw_inode)
    }

    /// Allocates up to `count` contiguous blocks.
    pub(super) fn alloc_blocks(&self, count: u32, goal: Ext2Bid) -> Result<Range<Ext2Bid>> {
        if count == 0 {
            return_errno_with_message!(Errno::EINVAL, "zero block allocation requested");
        }

        let mut sb = self.super_block.write();
        let nr_block_groups = sb.nr_block_groups() as usize;
        let sb_free_blocks = sb.free_blocks_count();
        let first_data_block = sb.first_data_block();
        let nr_blocks_per_group = sb.nr_blocks_per_group();
        if sb.free_blocks_count() == 0 {
            return_errno_with_message!(Errno::ENOSPC, "no free blocks on device");
        }

        if !Self::can_alloc(
            sb_free_blocks,
            sb.reserved_blocks_count(),
            sb.default_reserved_uid(),
            sb.default_reserved_gid(),
        ) {
            return_errno_with_message!(
                Errno::ENOSPC,
                "no free blocks available for unprivileged user"
            );
        }

        let goal_group = if goal > first_data_block {
            ((goal - first_data_block) / nr_blocks_per_group) as usize
        } else {
            0
        }
        .min(nr_block_groups - 1);

        for group_search_offset in 0..nr_block_groups {
            let group_idx = (goal_group + group_search_offset) % nr_block_groups;
            let group = &self.block_groups[group_idx];

            let range = group.alloc_blocks(count, sb_free_blocks)?;
            if !range.is_empty() {
                let allocated_count = range.end - range.start;
                sb.dec_free_blocks(allocated_count)?;
                return Ok(range);
            }
        }

        return_errno_with_message!(Errno::ENOSPC, "no free blocks available in any group");
    }

    /// Frees a range of blocks starting at `start`.
    pub(super) fn free_blocks(&self, start: Ext2Bid, count: u32) -> Result<()> {
        if count == 0 {
            return Ok(());
        }

        let mut sb = self.super_block.write();
        if !sb.is_data_block_valid(start, count) {
            return_errno_with_message!(Errno::EIO, "freeing invalid data block range");
        }
        let nr_blocks_per_group = sb.nr_blocks_per_group();
        let first_data_block = sb.first_data_block();

        let mut current_block = start;
        let mut remaining_blocks = count;

        while remaining_blocks > 0 {
            let group_idx = ((current_block - first_data_block) / nr_blocks_per_group) as usize;
            let group = &self.block_groups[group_idx];

            let group_first_block = group.first_block();
            let group_last_block = group.last_block();
            let group_size = group_last_block - group_first_block + 1;
            let group_start_bit = current_block - group_first_block;
            let blocks_in_group = remaining_blocks.min(group_size - group_start_bit);
            let freed_count =
                group.free_blocks(group_start_bit..(group_start_bit + blocks_in_group))?;
            if freed_count > 0 {
                sb.inc_free_blocks(freed_count)?;
            }
            current_block += blocks_in_group;
            remaining_blocks -= blocks_in_group;
        }

        Ok(())
    }

    /// Allocates and initializes a new inode.
    pub(super) fn create_inode(
        &self,
        parent_ino: Ext2Ino,
        inode_type: InodeType,
        perm: FilePerm,
    ) -> Result<Arc<Inode>> {
        if inode_type == InodeType::Unknown {
            return_errno_with_message!(Errno::EINVAL, "cannot create inode with unknown type");
        }

        let ino = self.alloc_ino(parent_ino, inode_type)?;
        let link_count = if inode_type.is_directory() { 2 } else { 1 };
        let (uid, gid) = Thread::current()
            .and_then(|thread| {
                thread
                    .as_posix_thread()
                    .map(|posix_thread| posix_thread.credentials())
            })
            .map(|credentials| {
                (
                    u32::from(credentials.fsuid()),
                    u32::from(credentials.fsgid()),
                )
            })
            .unwrap_or((0, 0));
        let now = utils::now();
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        let inode_desc = InodeDesc::new(inode_type, perm, uid, gid, link_count, generation, now);
        let raw_inode = RawInode::from(&inode_desc);

        let block_group = self
            .find_group(ino)
            .ok_or_else(|| Error::with_message(Errno::EIO, "block group index out of range"))?;

        if let Err(err) = self.write_back_inode_desc(ino, &raw_inode) {
            if let Ok(was_allocated) = block_group.free_inode(ino, inode_type)
                && was_allocated
            {
                let _ = self.super_block.write().inc_free_inodes();
            }

            return Err(err);
        }

        let block_group_idx = block_group.group_idx();
        Ok(Inode::new(
            ino,
            inode_desc.type_(),
            Dirty::new(inode_desc),
            block_group_idx,
            self.self_ref.clone(),
        ))
    }

    /// Frees an inode by number.
    pub(super) fn free_inode(&self, ino: Ext2Ino, inode_type: InodeType) -> Result<()> {
        let mut sb = self.super_block.write();
        let total_inodes = sb.total_inodes();
        let first_ino = sb.first_ino();
        if ino < first_ino || ino > total_inodes {
            return_errno_with_message!(Errno::EIO, "inode number out of valid range for free");
        }

        let group = self
            .find_group(ino)
            .ok_or_else(|| Error::with_message(Errno::EIO, "block group index out of range"))?;

        let was_allocated = group.free_inode(ino, inode_type)?;
        if was_allocated {
            sb.inc_free_inodes()?;
        }

        Ok(())
    }

    /// Submits an asynchronous block read starting at `bid`.
    pub(super) fn read_blocks_async(
        &self,
        bid: Ext2Bid,
        bio_segment: BioSegment,
        complete_fn: Option<BioCompleteFn>,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        self.block_device.read_blocks_async(
            Bid::new(bid as u64),
            bio_segment,
            complete_fn,
            io_batch,
        )?;
        Ok(())
    }

    /// Reads blocks synchronously starting at `bid`.
    pub(super) fn read_blocks(&self, bid: Ext2Bid, bio_segment: BioSegment) -> Result<()> {
        let bio_status = self
            .block_device
            .read_blocks(Bid::new(bid as u64), bio_segment)?;
        match bio_status {
            BioStatus::Complete => Ok(()),
            _ => {
                return_errno_with_message!(Errno::EIO, "failed to read blocks from block device")
            }
        }
    }

    /// Submits an asynchronous block write starting at `bid`.
    pub(super) fn write_blocks_async(
        &self,
        bid: Ext2Bid,
        bio_segment: BioSegment,
        complete_fn: Option<BioCompleteFn>,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        self.block_device.write_blocks_async(
            Bid::new(bid as u64),
            bio_segment,
            complete_fn,
            io_batch,
        )?;
        Ok(())
    }

    /// Writes blocks synchronously starting at `bid`.
    pub(super) fn write_blocks(&self, bid: Ext2Bid, bio_segment: BioSegment) -> Result<()> {
        let bio_status = self
            .block_device
            .write_blocks(Bid::new(bid as u64), bio_segment)?;
        match bio_status {
            BioStatus::Complete => Ok(()),
            _ => {
                return_errno_with_message!(Errno::EIO, "failed to write blocks to block device")
            }
        }
    }

    /// Syncs cached inodes and block-group-local metadata in all groups.
    pub(super) fn sync_all(&self) -> Result<()> {
        // `group_descriptors_segment` is updated without a filesystem-wide lock,
        // but each group writes only its own descriptor slice under its
        // `metadata.write()` guard.  Because groups are synced sequentially and
        // their descriptor offsets are disjoint, no two writers touch the same
        // bytes, so the segment is always consistent.
        for group in &self.block_groups {
            group.sync_all(&self.group_descriptors_segment)?;
        }
        self.sync_metadata()
    }

    /// Allocates a new inode number.
    fn alloc_ino(&self, parent_ino: Ext2Ino, inode_type: InodeType) -> Result<Ext2Ino> {
        let mut sb = self.super_block.write();
        let nr_block_groups = sb.nr_block_groups() as usize;
        let nr_inodes_per_group = sb.nr_inodes_per_group();
        let total_inodes = sb.total_inodes();
        if parent_ino < ROOT_INO || parent_ino > total_inodes {
            return_errno_with_message!(Errno::EIO, "parent inode number out of range");
        }
        if sb.free_inodes_count() == 0 {
            return_errno_with_message!(Errno::ENOSPC, "no free inodes on device");
        }

        let parent_group_idx = ((parent_ino - 1) / nr_inodes_per_group) as usize;
        for group_search_offset in 0..nr_block_groups {
            let group_idx = (parent_group_idx + group_search_offset) % nr_block_groups;
            let group = &self.block_groups[group_idx];
            let Some(inode_idx) = group.alloc_ino(inode_type)? else {
                continue;
            };

            let ino = (group_idx as u32) * nr_inodes_per_group + inode_idx + 1;
            if ino < sb.first_ino() || ino > total_inodes {
                return_errno_with_message!(Errno::EIO, "allocated inode number out of valid range");
            }
            sb.dec_free_inodes()?;

            return Ok(ino);
        }

        return_errno_with_message!(Errno::ENOSPC, "no free inodes available in any group");
    }

    /// Finds the block group that owns an inode number.
    fn find_group(&self, ino: Ext2Ino) -> Option<&BlockGroup> {
        debug_assert!(ino > 0);
        let group_idx = ((ino - 1) / self.nr_inodes_per_group) as usize;
        self.block_groups.get(group_idx)
    }

    /// Checks whether the current caller may allocate blocks.
    ///
    /// Non-privileged users are denied when free blocks fall below the reserved
    /// threshold, unless they have `CAP_SYS_RESOURCE` or match `s_resuid`/`s_resgid`.
    fn can_alloc(
        free_blocks: u32,
        reserved_blocks: u32,
        reserved_uid: u32,
        reserved_gid: u32,
    ) -> bool {
        if free_blocks > reserved_blocks {
            return true;
        }

        let Some(thread) = Thread::current() else {
            return true;
        };
        let Some(posix_thread) = thread.as_posix_thread() else {
            return true;
        };

        if lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
            UserNamespace::get_init_singleton().as_ref(),
            posix_thread,
            CapSet::SYS_RESOURCE,
        ))
        .is_ok()
        {
            return true;
        }

        let credentials = posix_thread.credentials();
        if u32::from(credentials.fsuid()) == reserved_uid {
            return true;
        }

        let reserved_gid_value = Gid::from(reserved_gid);
        if !reserved_gid_value.is_root() {
            if credentials.fsgid() == reserved_gid_value {
                return true;
            }
            if credentials.groups().contains(&reserved_gid_value) {
                return true;
            }
        }

        false
    }

    /// Flushes the superblock and all dirty group descriptors to the device.
    fn sync_metadata(&self) -> Result<()> {
        let mut sb_guard = self.super_block.write();

        let any_group_dirty = self.block_groups.iter().any(|group| group.is_desc_dirty());
        if !sb_guard.is_dirty() && !any_group_dirty {
            return Ok(());
        }

        let nr_block_groups = sb_guard.nr_block_groups() as usize;

        let mut total_free_blocks: u32 = 0;
        let mut total_free_inodes: u32 = 0;
        for group in &self.block_groups {
            total_free_blocks += group.free_blocks_count() as u32;
            total_free_inodes += group.free_inodes_count() as u32;
        }
        sb_guard.set_free_blocks_count(total_free_blocks);
        sb_guard.set_free_inodes_count(total_free_inodes);
        sb_guard.set_wtime(utils::now());

        let mut raw_sb = RawSuperBlock::from(&**sb_guard);
        self.write_sb_and_group_descs(
            &raw_sb,
            SUPER_BLOCK_OFFSET,
            sb_guard.group_descriptors_bid(0),
        )?;

        for group_idx in 1..nr_block_groups {
            if !sb_guard.is_backup_group(group_idx) {
                continue;
            }
            raw_sb.block_group_idx = group_idx as u16;
            self.write_sb_and_group_descs(
                &raw_sb,
                Bid::new(sb_guard.bid(group_idx) as u64).to_offset(),
                sb_guard.group_descriptors_bid(group_idx),
            )?;
        }

        sb_guard.clear_dirty();
        Ok(())
    }

    /// Persists one copy of the superblock and group descriptor table into the given block group.
    fn write_sb_and_group_descs(
        &self,
        raw_sb: &RawSuperBlock,
        sb_offset: usize,
        group_desc_bid: Ext2Bid,
    ) -> Result<()> {
        let group_desc_segment = self.group_descriptors_segment.clone();
        let bio_segment = BioSegment::new_from_segment(group_desc_segment, BioDirection::ToDevice);
        self.write_blocks(group_desc_bid, bio_segment)
            .map_err(|_| {
                Error::with_message(Errno::EIO, "failed to write group descriptor table")
            })?;
        if self
            .block_device
            .write_bytes(sb_offset, raw_sb.as_bytes())
            .is_err()
        {
            return_errno_with_message!(Errno::EIO, "failed to write superblock");
        }
        Ok(())
    }
}

#[cfg(ktest)]
mod test {

    use ostd::prelude::*;

    use super::*;
    use crate::{
        fs::{
            fs_impls::ext2::test_utils::{
                BlockBitmapInit, Ext2FixtureBuilder, Ext2MemoryDisk, InodeBitmapInit,
                RawInodeBuilder, assert_errno, create_file, default_fixture, make_valid_group_desc,
                make_valid_super_block,
            },
            vfs::file_system::FileSystem as FileSystemTrait,
        },
        time::clocks,
    };

    fn expected_overhead_blocks(sb: &SuperBlock) -> u32 {
        let nr_block_groups = sb.nr_block_groups() as usize;
        let gdb_count =
            ((nr_block_groups * size_of::<RawBlockGroup>()).div_ceil(BLOCK_SIZE)) as u32;
        let mut overhead = sb.first_data_block();

        for group_idx in 0..nr_block_groups {
            if group_idx == 0 || sb.is_backup_group(group_idx) {
                overhead += 1 + gdb_count;
            }
        }

        overhead + sb.nr_block_groups() * (2 + sb.nr_inode_table_blocks_per_group())
    }

    fn make_raw_inode(mode: u16, link_count: u16, dtime: u32) -> RawInode {
        RawInodeBuilder::new(mode)
            .link_count(link_count)
            .dtime(dtime)
            .build()
    }

    #[ktest]
    fn stat_bsddf_subtracts_overhead() {
        let f = Ext2FixtureBuilder::new(3, 512).build().unwrap();

        let stat = FileSystemTrait::sb(f.ext2.as_ref());
        let expected_overhead = expected_overhead_blocks(&f.sb);

        assert_eq!(
            stat.blocks,
            (f.sb.total_blocks() - expected_overhead) as usize
        );
        assert!(stat.blocks < f.sb.total_blocks() as usize);
    }

    #[ktest]
    fn stat_minixdf_reports_total() {
        let f = Ext2FixtureBuilder::new(3, 512).build().unwrap();
        let minixdf = CString::new("minixdf").unwrap();

        let ext2 = Ext2::open(
            f.disk.clone() as Arc<dyn BlockDevice>,
            Some(minixdf.as_c_str()),
        )
        .unwrap();

        let stat = FileSystemTrait::sb(ext2.as_ref());
        assert_eq!(stat.blocks, f.sb.total_blocks() as usize);
    }

    #[ktest]
    fn sync_metadata_writes_primary_and_backup() {
        clocks::init_for_ktest();
        let fixture = Ext2FixtureBuilder::new(3, 512)
            .with_free_blocks(0, 10)
            .with_free_inodes(16, 16)
            .build()
            .unwrap();
        let ext2 = &fixture.ext2;
        let disk = &fixture.disk;
        let sb = &fixture.sb;

        let _allocated_ino = ext2.alloc_ino(ROOT_INO, InodeType::File).unwrap();
        let expected_free_inodes = ext2.super_block().free_inodes_count();
        assert!(ext2.block_group(0).is_desc_dirty());

        ext2.sync_all().unwrap();

        assert!(!ext2.super_block().is_dirty());
        assert!(!ext2.block_group(0).is_desc_dirty());

        let nr_block_groups = sb.nr_block_groups() as usize;
        let desc_bytes = nr_block_groups * size_of::<RawBlockGroup>();
        let primary_desc_offset = Bid::new(sb.group_descriptors_bid(0) as u64).to_offset();

        let mut primary_desc = vec![0u8; desc_bytes];
        disk.segment()
            .read_bytes(primary_desc_offset, &mut primary_desc)
            .unwrap();
        let primary_sb = disk
            .segment()
            .read_val::<RawSuperBlock>(SUPER_BLOCK_OFFSET)
            .unwrap();
        assert_eq!(primary_sb.free_inodes_count, expected_free_inodes);

        for idx in 1..nr_block_groups {
            if !sb.is_backup_group(idx) {
                continue;
            }

            let backup_sb = disk
                .segment()
                .read_val::<RawSuperBlock>(Bid::new(sb.bid(idx) as u64).to_offset())
                .unwrap();
            assert_eq!(backup_sb.block_group_idx, idx as u16);

            let mut primary_cmp = primary_sb;
            let mut backup_cmp = backup_sb;
            primary_cmp.block_group_idx = 0;
            backup_cmp.block_group_idx = 0;
            assert_eq!(backup_cmp.as_bytes(), primary_cmp.as_bytes());

            let mut backup_desc = vec![0u8; desc_bytes];
            disk.segment()
                .read_bytes(
                    Bid::new(sb.group_descriptors_bid(idx) as u64).to_offset(),
                    &mut backup_desc,
                )
                .unwrap();
            assert_eq!(backup_desc, primary_desc);
        }
    }

    #[ktest]
    fn block_alloc_and_free_single_group_ok() {
        // Happy path: allocate a contiguous run and then free it back.
        let f = Ext2FixtureBuilder::new(1, 128)
            .with_free_blocks(31, 31)
            .block_bitmap(BlockBitmapInit::MetadataOnly)
            .build()
            .unwrap();

        let before_sb_free = f.ext2.super_block().free_blocks_count();
        let before_group_free = f.ext2.block_group(0).free_blocks_count();

        let goal = f.sb.group_first_block_no(0);
        let range = f.ext2.alloc_blocks(8, goal).unwrap();
        let alloc_len = range.end - range.start;
        assert!((1..=8).contains(&alloc_len));

        {
            let sb = f.ext2.super_block();
            assert!(sb.is_data_block_valid(range.start, alloc_len));
            let first_data = sb.first_data_block();
            let start_group = (range.start - first_data) / sb.nr_blocks_per_group();
            let end_group = (range.end - 1 - first_data) / sb.nr_blocks_per_group();
            assert_eq!(start_group, end_group);
        }

        assert_eq!(
            f.ext2.block_group(0).free_blocks_count(),
            before_group_free - alloc_len as u16
        );
        assert_eq!(
            f.ext2.super_block().free_blocks_count(),
            before_sb_free - alloc_len
        );

        f.ext2.free_blocks(range.start, alloc_len).unwrap();
        assert_eq!(f.ext2.block_group(0).free_blocks_count(), before_group_free);
        assert_eq!(f.ext2.super_block().free_blocks_count(), before_sb_free);
    }

    #[ktest]
    fn block_alloc_and_free_invalid_returns_err() {
        // No-space and invalid-request checks.
        let f_nospc = Ext2FixtureBuilder::new(1, 128)
            .with_free_blocks(0, 0)
            .block_bitmap(BlockBitmapInit::MetadataOnly)
            .build()
            .unwrap();
        assert_errno!(
            f_nospc.ext2.alloc_blocks(1, f_nospc.sb.first_data_block()),
            Errno::ENOSPC
        );
        assert_errno!(
            f_nospc.ext2.alloc_blocks(0, f_nospc.sb.first_data_block()),
            Errno::EINVAL
        );

        // Inconsistent counters/bitmap shape should surface as EIO on allocation.
        let f_corrupt = Ext2FixtureBuilder::new(1, 128)
            .with_free_blocks(1, 1)
            .block_bitmap(BlockBitmapInit::Full)
            .build()
            .unwrap();
        assert_errno!(
            f_corrupt
                .ext2
                .alloc_blocks(1, f_corrupt.sb.first_data_block()),
            Errno::EIO
        );

        // Free-path boundary and system-zone guards.
        let f_free = Ext2FixtureBuilder::new(1, 128)
            .with_free_blocks(31, 31)
            .block_bitmap(BlockBitmapInit::MetadataOnly)
            .build()
            .unwrap();
        assert!(f_free.ext2.free_blocks(10, 0).is_ok());
        assert_errno!(
            f_free.ext2.free_blocks(f_free.sb.first_data_block(), 1),
            Errno::EIO
        );
    }

    #[ktest]
    fn inode_alloc_and_free_single_group_ok() {
        // Allocate one directory inode and verify bitmap/counter transitions.
        let f = Ext2FixtureBuilder::new(1, 128)
            .with_free_inodes(16, 16)
            .inode_bitmap(InodeBitmapInit::ReservedOnly)
            .build()
            .unwrap();

        let before_sb_free = f.ext2.super_block().free_inodes_count();
        let before_group_free = f.ext2.block_group(0).free_inodes_count();
        let before_used_dirs = {
            let metadata = f.ext2.block_group(0).metadata();
            metadata.desc.used_dirs_count
        };

        let ino = f.ext2.alloc_ino(ROOT_INO, InodeType::Dir).unwrap();
        assert!(ino >= f.sb.first_ino() && ino <= f.sb.total_inodes());

        let bit = ((ino - 1) % f.sb.nr_inodes_per_group()) as u16;
        let metadata = f.ext2.block_group(0).metadata();
        assert!(metadata.inode_bitmap.is_allocated(bit));
        drop(metadata);
        assert_eq!(f.ext2.super_block().free_inodes_count(), before_sb_free - 1);
        assert_eq!(
            f.ext2.block_group(0).free_inodes_count(),
            before_group_free - 1
        );
        assert_eq!(
            f.ext2.block_group(0).metadata().desc.used_dirs_count,
            before_used_dirs + 1
        );

        // Free path now takes caller-provided inode type; no inode-table read is needed.
        let raw_dir = make_raw_inode(0o040755, 1, 0);
        f.ext2.write_back_inode_desc(ino, &raw_dir).unwrap();
        f.ext2.free_inode(ino, InodeType::Dir).unwrap();
        assert_eq!(f.ext2.super_block().free_inodes_count(), before_sb_free);
        assert_eq!(f.ext2.block_group(0).free_inodes_count(), before_group_free);
        assert_eq!(
            f.ext2.block_group(0).metadata().desc.used_dirs_count,
            before_used_dirs
        );
    }

    #[ktest]
    fn inode_alloc_and_free_invalid_returns_err() {
        // No free inode counter means ENOSPC without bitmap scan.
        let f_nospc = Ext2FixtureBuilder::new(1, 128)
            .with_free_inodes(0, 0)
            .inode_bitmap(InodeBitmapInit::ReservedOnly)
            .build()
            .unwrap();
        assert_errno!(
            f_nospc.ext2.alloc_ino(ROOT_INO, InodeType::File),
            Errno::ENOSPC
        );
        assert_errno!(
            f_nospc
                .ext2
                .alloc_ino(f_nospc.sb.total_inodes() + 1, InodeType::File),
            Errno::EIO
        );

        // All inode bitmap bits set -> no allocatable inode.
        let f_full = Ext2FixtureBuilder::new(1, 128)
            .with_free_inodes(8, 8)
            .inode_bitmap(InodeBitmapInit::Full)
            .build()
            .unwrap();
        assert_errno!(
            f_full.ext2.alloc_ino(ROOT_INO, InodeType::File),
            Errno::ENOSPC
        );

        let f_free = Ext2FixtureBuilder::new(1, 128)
            .with_free_inodes(8, 8)
            .inode_bitmap(InodeBitmapInit::ReservedOnly)
            .build()
            .unwrap();
        assert_errno!(
            f_free
                .ext2
                .free_inode(f_free.sb.first_ino() - 1, InodeType::File),
            Errno::EIO
        );

        // Already-free inode: should return Ok and keep counters unchanged.
        let target_ino = f_free.sb.first_ino();
        let raw_file = make_raw_inode(0o100644, 1, 0);
        f_free
            .ext2
            .write_back_inode_desc(target_ino, &raw_file)
            .unwrap();

        let before_sb = f_free.ext2.super_block().free_inodes_count();
        let before_group = f_free.ext2.block_group(0).free_inodes_count();
        f_free.ext2.free_inode(target_ino, InodeType::File).unwrap();
        assert_eq!(f_free.ext2.super_block().free_inodes_count(), before_sb);
        assert_eq!(f_free.ext2.block_group(0).free_inodes_count(), before_group);
    }

    #[ktest]
    fn load_group_descs_valid_image_ok() {
        let sb = make_valid_super_block(3);
        let descs = (0..sb.nr_block_groups() as usize)
            .map(|idx| make_valid_group_desc(&sb, idx))
            .collect::<Vec<_>>();
        let disk = Ext2MemoryDisk::new(64);
        disk.write_group_desc_table(&sb, &descs);

        let desc_offset = Bid::new(sb.group_descriptors_bid(0) as u64).to_offset();
        let first_desc = disk
            .segment()
            .read_val::<RawBlockGroup>(desc_offset)
            .unwrap();

        assert_eq!(first_desc.block_bitmap_bid, descs[0].block_bitmap_bid);
        assert_eq!(first_desc.inode_bitmap_bid, descs[0].inode_bitmap_bid);
        assert_eq!(first_desc.inode_table_bid, descs[0].inode_table_bid);
    }

    #[ktest]
    fn persist_and_reload_inode() {
        let (f, root) = default_fixture();

        let child = create_file(&root, "cache_file");
        let child_ino = child.ino();

        let cached = f.ext2.read_inode(child_ino).unwrap();
        assert!(Arc::ptr_eq(&child, &cached));

        drop(cached);
        drop(child);
        f.ext2.sync_all().unwrap();

        let metadata = f.ext2.block_group(0).metadata();
        assert!(metadata.inode_bitmap.is_allocated((child_ino - 1) as u16));
        drop(metadata);

        let reloaded = f.ext2.read_inode(child_ino).unwrap();
        assert_eq!(reloaded.ino(), child_ino);
    }

    #[ktest]
    fn load_block_bitmap_valid_image_ok() {
        let f = Ext2FixtureBuilder::new(2, 128)
            .block_bitmap(BlockBitmapInit::MetadataOnly)
            .build()
            .unwrap();
        let group = f.ext2.block_group(0);
        let first = f.sb.group_first_block_no(0);

        let metadata = group.metadata();
        // Block bitmap, inode bitmap, and inode table blocks must be marked.
        let bb = (f.descs[0].block_bitmap_bid - first) as u16;
        let ib = (f.descs[0].inode_bitmap_bid - first) as u16;
        let it = (f.descs[0].inode_table_bid - first) as u16;
        assert!(metadata.block_bitmap.is_allocated(bb));
        assert!(metadata.block_bitmap.is_allocated(ib));
        assert!(metadata.block_bitmap.is_allocated(it));
    }
}
