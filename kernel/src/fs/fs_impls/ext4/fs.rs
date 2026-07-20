// SPDX-License-Identifier: MPL-2.0

//! The `Ext4` filesystem object: mount, geometry, block allocation, and inode
//! lookup.
//!
//! Mutable filesystem metadata uses dirty tracking.
//! Each block group caches its allocation bitmaps and loaded inodes, while
//! `Ext4` coordinates allocation and persists the resulting metadata updates.

use core::sync::atomic::{AtomicU32, Ordering};

use device_id::DeviceId;

use super::{
    block_group::BlockGroup,
    feature::{self, FeatureIncompatSet},
    inode::{FilePerm, Inode, InodeDesc, RawInode},
    prelude::*,
    super_block::{RawSuperBlock, SUPER_BLOCK_OFFSET, SuperBlock},
    utils,
};
use crate::{
    fs::vfs::file_system::FsEventSubscriberStats,
    process::{Gid, UserNamespace, credentials::capabilities::CapSet, posix_thread::AsPosixThread},
    security::lsm::hooks as lsm_hooks,
    thread::Thread,
};

/// Root directory inode number.
pub(super) const ROOT_INO: Ext4Ino = 2;

/// An ext4 filesystem instance.
pub struct Ext4 {
    block_device: Arc<dyn BlockDevice>,
    /// Superblock with dirty tracking.
    super_block: RwMutex<Dirty<SuperBlock>>,
    /// Per-group block-side metadata (descriptor + block bitmap).
    block_groups: Vec<BlockGroup>,
    /// Inodes per group, cached once at mount to avoid locking `super_block` on
    /// the inode read path.
    nr_inodes_per_group: u32,
    /// The filesystem-type name this instance was mounted as.
    flavor: MountFlavor,
    /// Runtime mount options that affect block-count reporting.
    mount_options: ExtMountOptions,
    /// Monotonic source for the `i_generation` stamped onto each newly created
    /// inode. Seeded from the mount time, like ext2.
    next_generation: AtomicU32,
    fs_event_subscriber_stats: FsEventSubscriberStats,
    self_ref: Weak<Ext4>,
}

/// Which filesystem-type name this instance was mounted as.
///
/// The unified driver registers both the `ext2` and the `ext4` type names.
/// The flavor controls the mount-time feature validation (an `ext2` mount
/// only accepts true ext2-format volumes, like Linux's `IS_EXT2_SB` rule)
/// and the name reported back to the VFS.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MountFlavor {
    Ext2,
    Ext4,
}

impl MountFlavor {
    /// The filesystem-type name this flavor was mounted as.
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Ext2 => "ext2",
            Self::Ext4 => "ext4",
        }
    }
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
struct ExtMountOptions {
    stat_block_accounting: StatBlockAccounting,
}

impl ExtMountOptions {
    /// Parses mount options that control block-count reporting.
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

impl Ext4 {
    /// Mounts a volume from a block device under the given type name.
    pub(super) fn open(
        device: Arc<dyn BlockDevice>,
        flavor: MountFlavor,
        data: Option<&CStr>,
    ) -> Result<Arc<Self>> {
        let raw_super_block = device.read_val::<RawSuperBlock>(SUPER_BLOCK_OFFSET)?;
        let super_block = SuperBlock::try_from(raw_super_block)?;
        feature::check_flavor(&super_block, flavor)?;
        let nr_inodes_per_group = super_block.nr_inodes_per_group();

        let block_groups = Self::load_block_groups(device.clone(), &super_block)?;

        Ok(Arc::new_cyclic(|weak| Ext4 {
            block_device: device,
            super_block: RwMutex::new(Dirty::new(super_block)),
            block_groups,
            nr_inodes_per_group,
            flavor,
            mount_options: ExtMountOptions::parse(data),
            next_generation: AtomicU32::new(
                u32::try_from(utils::now().as_secs() & u64::from(u32::MAX))
                    .expect("masked generation fits u32"),
            ),
            fs_event_subscriber_stats: FsEventSubscriberStats::new(),
            self_ref: weak.clone(),
        }))
    }

    /// Loads every block group from the descriptor table, which immediately
    /// follows the block holding the superblock.
    fn load_block_groups(
        device: Arc<dyn BlockDevice>,
        super_block: &SuperBlock,
    ) -> Result<Vec<BlockGroup>> {
        let nr_groups = usize::try_from(super_block.nr_block_groups())
            .map_err(|_| Error::with_message(Errno::EOVERFLOW, "block group count overflow"))?;
        let gdt_block = super_block
            .first_data_block()
            .checked_add(1)
            .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "descriptor block overflow"))?;
        let gdt_base_offset = utils::block_offset(gdt_block, super_block.block_size())?;

        let mut block_groups = Vec::with_capacity(nr_groups);
        for group_idx in 0..nr_groups {
            let group = BlockGroup::load(device.clone(), group_idx, super_block, gdt_base_offset)?;
            block_groups.push(group);
        }
        Ok(block_groups)
    }

    pub(super) fn block_device(&self) -> &Arc<dyn BlockDevice> {
        &self.block_device
    }

    /// Returns whether `statfs` reports totals in the `minixdf` style
    /// (including metadata overhead).
    pub(super) fn uses_minix_df(&self) -> bool {
        self.mount_options.stat_block_accounting == StatBlockAccounting::IncludeOverhead
    }

    /// Returns the filesystem-type name this instance was mounted as.
    pub(super) fn flavor(&self) -> MountFlavor {
        self.flavor
    }

    /// Returns a reference to the block group at `group_idx`.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(super) fn block_group(&self, group_idx: usize) -> &BlockGroup {
        &self.block_groups[group_idx]
    }

    /// Returns a read guard of the superblock.
    pub(super) fn super_block(&self) -> RwMutexReadGuard<'_, Dirty<SuperBlock>> {
        self.super_block.read()
    }

    /// Returns the device ID of the backing block device.
    pub(super) fn container_device_id(&self) -> DeviceId {
        self.block_device.id()
    }

    pub(super) fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        &self.fs_event_subscriber_stats
    }

    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(super) fn this(&self) -> Weak<Ext4> {
        self.self_ref.clone()
    }

    /// Reads the root directory inode.
    pub(super) fn root_inode(&self) -> Result<Arc<Inode>> {
        self.read_inode(ROOT_INO)
    }

    /// Reads an inode, returning the cached `Arc<Inode>` if one already exists.
    ///
    /// Routing through the owning group's inode cache gives every reader of one
    /// inode number the same in-memory inode (identity), which the
    /// filesystem-level sync relies on to enumerate and flush all dirty inodes.
    pub(super) fn read_inode(&self, ino: Ext4Ino) -> Result<Arc<Inode>> {
        self.find_group(ino)?
            .lookup_inode(ino, self.self_ref.clone())
    }

    /// Inserts a newly created inode into its block group's live cache.
    pub(super) fn insert_inode(&self, inode: Arc<Inode>) {
        if let Ok(group) = self.find_group(inode.ino()) {
            group.insert_inode(inode);
        }
    }

    /// Removes an inode from its block group's live cache.
    pub(super) fn remove_inode(&self, ino: Ext4Ino) -> Option<Arc<Inode>> {
        self.find_group(ino)
            .ok()
            .and_then(|group| group.remove_inode(ino))
    }

    /// Returns whether `ino` is marked allocated in its owning group's inode
    /// bitmap. Used by the reclaim path to skip an already-freed inode. Mirrors
    /// ext2 routing through the owning block group.
    pub(super) fn is_inode_allocated(&self, ino: Ext4Ino) -> bool {
        self.find_group(ino)
            .map(|group| group.is_inode_allocated(ino))
            .unwrap_or(false)
    }

    /// Read-modify-writes the on-disk `RawInode` for `ino`, patching only the
    /// fields that buffered writes can mutate (size, `i_blocks`, the extent
    /// root, timestamps, flags, and link count) and preserving everything else
    /// (`extra_isize`, checksums, generation, xattr tail, osd fields) losslessly.
    pub(super) fn write_back_inode_desc(
        &self,
        ino: Ext4Ino,
        desc: &InodeDesc,
        root: &[u32; super::inode::RAW_BLOCK_PTRS_LEN],
    ) -> Result<()> {
        let (offset, inode_size) = self.inode_slot_geometry(ino)?;
        // Hold the group's inode-table lock across the whole slot RMW: the
        // block layer rewrites the surrounding sector, so an unserialized
        // concurrent write to a neighboring slot would be lost.
        let group = self.find_group(ino)?;
        let _slot_guard = group.lock_inode_table();
        let mut raw = RawInode::read_from_slot(&self.block_device, offset, inode_size)?;

        raw.update_from_desc(desc, root)?;

        raw.write_to_slot(&self.block_device, offset, inode_size)?;
        Ok(())
    }

    /// Computes the device byte offset and on-disk slot size of the
    /// `RawInode` for `ino` in its group's inode table.
    fn inode_slot_geometry(&self, ino: Ext4Ino) -> Result<(usize, usize)> {
        if ino == 0 {
            return_errno_with_message!(Errno::ENOENT, "invalid inode number 0");
        }
        let group_idx = usize::try_from((ino - 1) / self.nr_inodes_per_group)
            .expect("inode group index fits usize");
        let idx_in_group =
            usize::try_from((ino - 1) % self.nr_inodes_per_group).expect("inode index fits usize");
        let group = self
            .block_groups
            .get(group_idx)
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "inode block group out of range"))?;
        let sb = self.super_block.read();
        let inode_table_offset = utils::block_offset(group.inode_table_bid(), sb.block_size())?;
        let inode_slot_offset = idx_in_group
            .checked_mul(sb.inode_size())
            .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "inode slot offset overflow"))?;
        let offset = inode_table_offset
            .checked_add(inode_slot_offset)
            .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "inode table offset overflow"))?;
        Ok((offset, sb.inode_size()))
    }

    /// Writes the complete on-disk `RawInode` for a freshly created inode.
    ///
    /// Unlike [`write_back_inode_desc`](Self::write_back_inode_desc), which is a
    /// read-modify-write tuned for the buffered-write path (and therefore
    /// preserves the on-disk type bits and generation), this writes every field
    /// of a brand-new inode from scratch: the type/permission mode, owners,
    /// timestamps, generation, the inline extent root, flags, link count, and
    /// `extra_isize`. The previous slot contents (a deleted inode or zeros) are
    /// fully overwritten.
    fn write_new_inode_desc(&self, ino: Ext4Ino, desc: &InodeDesc) -> Result<()> {
        let (offset, inode_size) = self.inode_slot_geometry(ino)?;

        let raw = RawInode::from_desc(desc)?;

        // Serialize with other slot writes in this group (see
        // `write_back_inode_desc` for why).
        let group = self.find_group(ino)?;
        let _slot_guard = group.lock_inode_table();
        raw.write_to_slot(&self.block_device, offset, inode_size)?;
        Ok(())
    }

    /// Allocates up to `count` contiguous blocks, preferring the group that owns
    /// `goal`.
    ///
    /// Searches groups in a ring starting from the goal group. Returns
    /// `Err(ENOSPC)` if no group can satisfy the request, `Err(EINVAL)` if
    /// `count` is zero.
    pub(super) fn alloc_blocks(&self, count: u32, goal: Ext4Bid) -> Result<Range<Ext4Bid>> {
        if count == 0 {
            return_errno_with_message!(Errno::EINVAL, "zero block allocation requested");
        }

        let mut sb = self.super_block.write();
        let nr_block_groups = usize::try_from(sb.nr_block_groups())
            .map_err(|_| Error::with_message(Errno::EOVERFLOW, "block group count overflow"))?;
        let sb_free_blocks = sb.free_blocks_count();
        let first_data_block = sb.first_data_block();
        let nr_blocks_per_group = Ext4Bid::from(sb.nr_blocks_per_group());
        if sb_free_blocks == 0 {
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
            usize::try_from((goal - first_data_block) / nr_blocks_per_group).unwrap_or(usize::MAX)
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

    /// Frees `count` blocks starting at `start`, splitting across groups.
    pub(super) fn free_blocks(&self, start: Ext4Bid, count: u32) -> Result<()> {
        if count == 0 {
            return Ok(());
        }

        let mut sb = self.super_block.write();
        let nr_blocks_per_group = Ext4Bid::from(sb.nr_blocks_per_group());
        let first_data_block = sb.first_data_block();
        if start < first_data_block {
            return_errno_with_message!(Errno::EINVAL, "block range starts before data blocks");
        }
        let end = start
            .checked_add(Ext4Bid::from(count))
            .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "block range overflow"))?;
        if end > sb.total_blocks() {
            return_errno_with_message!(Errno::EINVAL, "block range outside filesystem");
        }

        let mut current_block = start;
        let mut remaining_blocks = count;

        while remaining_blocks > 0 {
            let group_idx = usize::try_from(
                (current_block - first_data_block) / nr_blocks_per_group,
            )
            .map_err(|_| Error::with_message(Errno::EOVERFLOW, "block group index overflow"))?;
            let group = self.block_groups.get(group_idx).ok_or_else(|| {
                Error::with_message(Errno::EINVAL, "block range outside filesystem")
            })?;

            let group_first_block = group.first_block();
            let group_last_block = group.last_block();
            if current_block < group_first_block || current_block > group_last_block {
                return_errno_with_message!(Errno::EINVAL, "block is outside its group");
            }
            let group_size = u32::try_from(group_last_block - group_first_block + 1)
                .map_err(|_| Error::with_message(Errno::EOVERFLOW, "block group is too large"))?;
            let group_start_bit = u32::try_from(current_block - group_first_block)
                .map_err(|_| Error::with_message(Errno::EINVAL, "block is outside its group"))?;
            let blocks_in_group = remaining_blocks.min(group_size - group_start_bit);
            let freed_count =
                group.free_blocks(group_start_bit..(group_start_bit + blocks_in_group))?;
            if freed_count > 0 {
                sb.inc_free_blocks(u64::from(freed_count))?;
            }
            current_block = current_block
                .checked_add(Ext4Bid::from(blocks_in_group))
                .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "block range overflow"))?;
            remaining_blocks -= blocks_in_group;
        }

        Ok(())
    }

    /// Allocates and initializes a new inode, returning the live `Arc<Inode>`.
    ///
    /// Allocates an inode number, builds a fresh [`InodeDesc`] (an empty extent
    /// root with the `EXTENTS` flag set, size/`i_blocks` 0, owners from the
    /// caller's fsuid/fsgid, `now` timestamps, and a monotonic generation), and
    /// writes the full on-disk inode. On failure, the allocation is rolled back.
    pub(super) fn create_inode(
        &self,
        parent_ino: Ext4Ino,
        type_: InodeType,
        perm: FilePerm,
    ) -> Result<Arc<Inode>> {
        let ino = self.alloc_ino(parent_ino, type_)?;

        let link_count = if type_.is_directory() { 2 } else { 1 };
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
        // New inodes follow the volume's mapping format: extent volumes get
        // extent roots, ext2-format volumes get indirect pointer arrays, so a
        // volume written by this driver keeps its original format. Special
        // files are excepted: their `i_block` holds the device encoding (or
        // nothing), never a mapping root, so they must not carry the `EXTENTS`
        // flag even on an extent volume (Linux does the same).
        let extent_based = matches!(type_, InodeType::File | InodeType::Dir | InodeType::SymLink)
            && self
                .super_block
                .read()
                .feature_incompat()
                .contains(FeatureIncompatSet::EXTENTS);
        let inode_desc = InodeDesc::new(
            type_,
            perm,
            uid,
            gid,
            link_count,
            generation,
            now,
            extent_based,
        );

        if let Err(err) = self.write_new_inode_desc(ino, &inode_desc) {
            // Roll back the inode allocation: clear the bitmap bit and restore
            // the superblock free-inode counter.
            if let Err(free_err) = self.free_inode(ino, type_) {
                error!("create_inode: rollback free_inode failed: {:?}", free_err);
            }
            return Err(err);
        }

        let block_group_idx = usize::try_from((ino - 1) / self.nr_inodes_per_group)
            .expect("inode group index fits usize");
        Inode::new(
            ino,
            inode_desc.type_(),
            Dirty::new(inode_desc),
            block_group_idx,
            self.self_ref.clone(),
        )
    }

    /// Frees an inode by number, mirroring [`Self::free_blocks`] on the block side.
    pub(super) fn free_inode(&self, ino: Ext4Ino, type_: InodeType) -> Result<()> {
        let mut sb = self.super_block.write();
        let group = self.find_group(ino)?;
        let local_idx = (ino - 1) % self.nr_inodes_per_group;

        let was_allocated = group.free_inode(local_idx, type_)?;
        if was_allocated {
            sb.inc_free_inodes()?;
        }

        Ok(())
    }

    /// Submits an asynchronous read of one or more blocks starting at `bid`.
    pub(super) fn read_blocks_async(
        &self,
        bid: Ext4Bid,
        bio_segment: BioSegment,
        complete_fn: Option<BioCompleteFn>,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        self.block_device
            .read_blocks_async(Bid::new(bid), bio_segment, complete_fn, io_batch)?;
        Ok(())
    }

    /// Reads one or more blocks synchronously starting at `bid`.
    pub(super) fn read_blocks(&self, bid: Ext4Bid, bio_segment: BioSegment) -> Result<()> {
        let bio_status = self.block_device.read_blocks(Bid::new(bid), bio_segment)?;
        match bio_status {
            BioStatus::Complete => Ok(()),
            _ => {
                return_errno_with_message!(Errno::EIO, "failed to read blocks from block device")
            }
        }
    }

    /// Submits an asynchronous write of one or more blocks starting at `bid`.
    pub(super) fn write_blocks_async(
        &self,
        bid: Ext4Bid,
        bio_segment: BioSegment,
        complete_fn: Option<BioCompleteFn>,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        self.block_device
            .write_blocks_async(Bid::new(bid), bio_segment, complete_fn, io_batch)?;
        Ok(())
    }

    /// Writes one or more blocks synchronously starting at `bid`.
    pub(super) fn write_blocks(&self, bid: Ext4Bid, bio_segment: BioSegment) -> Result<()> {
        let bio_status = self.block_device.write_blocks(Bid::new(bid), bio_segment)?;
        match bio_status {
            BioStatus::Complete => Ok(()),
            _ => {
                return_errno_with_message!(Errno::EIO, "failed to write blocks to block device")
            }
        }
    }

    /// Flushes every cached inode together with the block-side metadata.
    ///
    /// Order matters for on-disk consistency: each group's cached inodes (their
    /// data pages + inode-table descriptors) are flushed first, then the dirty
    /// bitmaps/GDT/superblock. Flushing inodes before the bitmap keeps the
    /// on-disk extents and the block bitmap mutually consistent — otherwise a
    /// truncate that freed blocks in the bitmap could be persisted while the
    /// inode still on disk references those (now free) blocks, which `e2fsck`
    /// reports as corruption.
    ///
    /// Inode flushing never holds a group's `inode_cache` lock across the sync
    /// (see [`BlockGroup::sync_inodes`]), so the only locks held in sequence are
    /// `inode.inner.write()` then, later, `super_block.write()` + per-group
    /// `metadata.write()` — no inversion.
    pub(super) fn sync_all(&self) -> Result<()> {
        for group in &self.block_groups {
            group.sync_inodes()?;
        }
        self.sync_metadata()
    }

    /// Allocates one inode, preferring the group that owns `parent_ino`.
    ///
    /// Searches groups in a ring starting from the parent's group. Returns the
    /// global inode number on success, or `ENOSPC` if no group has a free inode.
    pub(super) fn alloc_ino(&self, parent_ino: Ext4Ino, type_: InodeType) -> Result<Ext4Ino> {
        if type_ == InodeType::Unknown {
            return_errno_with_message!(Errno::EINVAL, "cannot allocate inode with unknown type");
        }

        let mut sb = self.super_block.write();
        let nr_block_groups = usize::try_from(sb.nr_block_groups())
            .map_err(|_| Error::with_message(Errno::EOVERFLOW, "block group count overflow"))?;
        let nr_inodes_per_group = sb.nr_inodes_per_group();
        let total_inodes = sb.total_inodes();
        if parent_ino < ROOT_INO || parent_ino > total_inodes {
            return_errno_with_message!(Errno::EIO, "parent inode number out of range");
        }
        if sb.free_inodes_count() == 0 {
            return_errno_with_message!(Errno::ENOSPC, "no free inodes on device");
        }

        let parent_group = usize::try_from((parent_ino - 1) / nr_inodes_per_group)
            .expect("inode group index fits usize");
        for group_search_offset in 0..nr_block_groups {
            let group_idx = (parent_group + group_search_offset) % nr_block_groups;
            let group = &self.block_groups[group_idx];

            let Some(local_idx) = group.alloc_ino(type_)? else {
                continue;
            };

            let group_idx = u32::try_from(group_idx)
                .map_err(|_| Error::with_message(Errno::EOVERFLOW, "inode group index overflow"))?;
            let ino = group_idx
                .checked_mul(nr_inodes_per_group)
                .and_then(|base| base.checked_add(local_idx))
                .and_then(|base| base.checked_add(1))
                .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "inode number overflow"))?;
            if ino < sb.first_ino() || ino > total_inodes {
                // Roll back the group-level allocation before erroring out.
                let _ = group.free_inode(local_idx, type_);
                return_errno_with_message!(Errno::EIO, "allocated inode number out of valid range");
            }
            sb.dec_free_inodes()?;

            return Ok(ino);
        }

        return_errno_with_message!(Errno::ENOSPC, "no free inodes available in any group");
    }

    /// Returns the block group that owns `ino`.
    fn find_group(&self, ino: Ext4Ino) -> Result<&BlockGroup> {
        if ino == 0 {
            return_errno_with_message!(Errno::ENOENT, "invalid inode number 0");
        }
        let group_idx = usize::try_from((ino - 1) / self.nr_inodes_per_group)
            .expect("inode group index fits usize");
        self.block_groups
            .get(group_idx)
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "inode block group out of range"))
    }

    /// Checks whether the current caller may allocate blocks.
    ///
    /// Non-privileged users are denied when free blocks fall below the reserved
    /// threshold, unless they have `CAP_SYS_RESOURCE` or match `s_resuid`/`s_resgid`.
    fn can_alloc(
        free_blocks: u64,
        reserved_blocks: u64,
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

    /// Writes back the superblock and every dirty group descriptor/bitmap.
    pub(super) fn sync_metadata(&self) -> Result<()> {
        for group in &self.block_groups {
            group.sync_metadata()?;
        }

        let mut sb = self.super_block.write();
        if sb.is_dirty() {
            // RMW the on-disk superblock: patch only the free-block and
            // free-inode counters so every other on-disk field is preserved
            // losslessly.
            let mut raw = self
                .block_device
                .read_val::<RawSuperBlock>(SUPER_BLOCK_OFFSET)
                .map_err(|_| {
                    Error::with_message(Errno::EIO, "failed to read superblock for sync")
                })?;
            raw.free_blocks_count = u32::try_from(sb.free_blocks_count()).map_err(|_| {
                Error::with_message(Errno::EOVERFLOW, "free block count exceeds disk field")
            })?;
            raw.free_inodes_count = sb.free_inodes_count();
            // Stamp the write time, like Linux. Only the primary superblock is
            // written: backup copies are maintained by mkfs/fsck/resize, not
            // by the running filesystem.
            raw.wtime = utils::now().into();
            self.block_device
                .write_val(SUPER_BLOCK_OFFSET, &raw)
                .map_err(|_| Error::with_message(Errno::EIO, "failed to write superblock"))?;
            sb.clear_dirty();
        }

        Ok(())
    }
}

/// Test-only rollback guard for blocks allocated through [`Ext4::alloc_blocks`].
#[cfg(ktest)]
pub(super) struct FsBlockAllocGuard<'a> {
    fs: &'a Ext4,
    ranges: Vec<Range<Ext4Bid>>,
}

#[cfg(ktest)]
impl<'a> FsBlockAllocGuard<'a> {
    /// Creates a guard tracking no ranges yet.
    pub(super) fn new(fs: &'a Ext4) -> Self {
        Self {
            fs,
            ranges: Vec::new(),
        }
    }

    /// Records an allocated range to be rolled back on drop.
    pub(super) fn extend(&mut self, range: Range<Ext4Bid>) {
        if !range.is_empty() {
            self.ranges.push(range);
        }
    }
}

#[cfg(ktest)]
impl Drop for FsBlockAllocGuard<'_> {
    fn drop(&mut self) {
        for range in self.ranges.iter() {
            let count = (range.end - range.start) as u32;
            if let Err(err) = self.fs.free_blocks(range.start, count) {
                error!(
                    "FsBlockAllocGuard: failed to free range {:?} in rollback: {:?}",
                    range, err
                );
            }
        }
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    use super::{
        super::{block_group::RawBlockGroup, test_utils::Ext4FixtureBuilder},
        *,
    };
    use crate::fs::vfs::file_system::FileSystem as FileSystemTrait;

    #[ktest]
    fn alloc_and_free_single_group_ok() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048)
            .with_block_bitmap_metadata_marked()
            .build()
            .unwrap();

        let before_sb_free = f.ext4.super_block().free_blocks_count();
        let before_group_free = f.ext4.block_group(0).free_blocks_count();

        let goal = f.ext4.block_group(0).first_block();
        let range = f.ext4.alloc_blocks(8, goal).unwrap();
        let alloc_len = (range.end - range.start) as u32;
        assert!((1..=8).contains(&alloc_len));

        // The whole run lives in a single group.
        let group = f.ext4.block_group(0);
        assert!(range.start >= group.first_block());
        assert!(range.end - 1 <= group.last_block());

        assert_eq!(
            f.ext4.block_group(0).free_blocks_count(),
            before_group_free - alloc_len
        );
        assert_eq!(
            f.ext4.super_block().free_blocks_count(),
            before_sb_free - alloc_len as u64
        );

        f.ext4.free_blocks(range.start, alloc_len).unwrap();
        assert_eq!(f.ext4.block_group(0).free_blocks_count(), before_group_free);
        assert_eq!(f.ext4.super_block().free_blocks_count(), before_sb_free);
    }

    #[ktest]
    fn block_alloc_guard_rolls_back_on_drop() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048)
            .with_block_bitmap_metadata_marked()
            .build()
            .unwrap();
        let before_sb_free = f.ext4.super_block().free_blocks_count();
        let before_group_free = f.ext4.block_group(0).free_blocks_count();

        let range = f
            .ext4
            .alloc_blocks(4, f.ext4.block_group(0).first_block())
            .unwrap();
        let alloc_len = (range.end - range.start) as u32;
        assert!(alloc_len > 0);

        {
            let mut guard = FsBlockAllocGuard::new(&f.ext4);
            guard.extend(range.clone());
            // Drop without commit -> rollback.
        }

        // Counts restored and bitmap bits cleared.
        assert_eq!(f.ext4.super_block().free_blocks_count(), before_sb_free);
        assert_eq!(f.ext4.block_group(0).free_blocks_count(), before_group_free);
        let group = f.ext4.block_group(0);
        let metadata = group.metadata();
        for bid in range.clone() {
            let bit = (bid - group.first_block()) as u16;
            assert!(!metadata.block_bitmap.is_allocated(bit));
        }
    }

    #[ktest]
    fn sync_metadata_round_trip_lossless() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048)
            .with_block_bitmap_metadata_marked()
            .build()
            .unwrap();

        // Snapshot the raw group descriptor before any mutation.
        let gdt_offset = (f.ext4.super_block().first_data_block() as usize + 1) * BLOCK_SIZE;
        let raw_before = f
            .disk
            .segment()
            .read_val::<RawBlockGroup>(gdt_offset)
            .unwrap();
        let raw_sb_before = f
            .disk
            .segment()
            .read_val::<RawSuperBlock>(SUPER_BLOCK_OFFSET)
            .unwrap();

        let range = f
            .ext4
            .alloc_blocks(4, f.ext4.block_group(0).first_block())
            .unwrap();
        let alloc_len = (range.end - range.start) as u32;
        f.ext4.sync_metadata().unwrap();

        let raw_after = f
            .disk
            .segment()
            .read_val::<RawBlockGroup>(gdt_offset)
            .unwrap();
        let raw_sb_after = f
            .disk
            .segment()
            .read_val::<RawSuperBlock>(SUPER_BLOCK_OFFSET)
            .unwrap();

        // Only free-block counters changed; all other fields are preserved.
        assert_eq!(
            raw_after.free_blocks_count_lo,
            raw_before.free_blocks_count_lo - alloc_len as u16
        );
        assert_eq!(raw_after.inode_table_lo, raw_before.inode_table_lo);
        assert_eq!(raw_after.block_bitmap_lo, raw_before.block_bitmap_lo);
        assert_eq!(raw_after.inode_bitmap_lo, raw_before.inode_bitmap_lo);
        assert_eq!(raw_after.flags, raw_before.flags);
        assert_eq!(raw_after.checksum, raw_before.checksum);
        assert_eq!(raw_after.itable_unused_lo, raw_before.itable_unused_lo);
        assert_eq!(
            raw_after.free_inodes_count_lo,
            raw_before.free_inodes_count_lo
        );

        assert_eq!(
            raw_sb_after.free_blocks_count,
            raw_sb_before.free_blocks_count - alloc_len
        );
        assert_eq!(raw_sb_after.inodes_count, raw_sb_before.inodes_count);
        assert_eq!(raw_sb_after.blocks_count, raw_sb_before.blocks_count);
        assert_eq!(raw_sb_after.magic, raw_sb_before.magic);
    }

    use super::super::test_utils::make_empty_file_inode;

    const SECTORS_PER_BLOCK: u64 = (BLOCK_SIZE / SECTOR_SIZE) as u64;

    fn write_all(inode: &Inode, offset: usize, data: &[u8]) {
        let mut reader = VmReader::from(data).to_fallible();
        let n = inode.write_at(offset, &mut reader).unwrap();
        assert_eq!(n, data.len());
    }

    /// Returns whether physical block `pblock` is marked allocated in group 0.
    fn block_is_allocated(f: &super::super::test_utils::Ext4Fixture, pblock: Ext4Bid) -> bool {
        let group = f.ext4.block_group(0);
        group
            .metadata()
            .block_bitmap
            .is_allocated((pblock - group.first_block()) as u16)
    }

    /// Parses the inline depth-0 extent root and returns the physical block that
    /// logical block `lblock` maps to, if any. Only handles the single-contiguous
    /// extent shape these tests build.
    fn ondisk_pblock_of(raw: &RawInode, lblock: u32) -> Option<Ext4Bid> {
        let entries = (raw.block[0] >> 16) & 0xFFFF;
        let depth = (raw.block[1] >> 16) & 0xFFFF;
        if depth != 0 {
            return None;
        }
        for i in 0..entries as usize {
            let ee_block = raw.block[3 + i * 3];
            let raw_len = raw.block[4 + i * 3] & 0xFFFF;
            // An unwritten extent biases its length by 32768; mask it off.
            let len = if raw_len > 32768 {
                raw_len - 32768
            } else {
                raw_len
            };
            let ee_start = raw.block[5 + i * 3] as Ext4Bid;
            if lblock >= ee_block && lblock < ee_block + len {
                return Some(ee_start + (lblock - ee_block) as Ext4Bid);
            }
        }
        None
    }

    /// `read_inode` returns the same `Arc<Inode>` for repeated reads of one ino:
    /// the per-group inode cache gives the inode a stable identity.
    #[ktest]
    fn read_inode_returns_same_arc_identity() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048)
            .with_block_bitmap_metadata_marked()
            .build()
            .unwrap();
        f.write_raw_inode(11, &make_empty_file_inode());

        let a = f.ext4.read_inode(11).unwrap();
        let b = f.ext4.read_inode(11).unwrap();
        assert!(Arc::ptr_eq(&a, &b));

        // A different inode is a different identity.
        f.write_raw_inode(12, &make_empty_file_inode());
        let c = f.ext4.read_inode(12).unwrap();
        assert!(!Arc::ptr_eq(&a, &c));
    }

    /// `fs.sync_all()` flushes a dirty inode's metadata to the on-disk inode
    /// table: after a write + `sync_all`, the raw inode read straight from the
    /// device segment carries the new size and `i_blocks`.
    #[ktest]
    fn sync_all_flushes_dirty_inode() {
        crate::time::clocks::init_for_ktest();
        let f = Ext4FixtureBuilder::new(2048, 256, 2048)
            .with_block_bitmap_metadata_marked()
            .build()
            .unwrap();
        f.write_raw_inode(11, &make_empty_file_inode());

        let inode = f.ext4.read_inode(11).unwrap();
        write_all(&inode, 0, &[0xAB; BLOCK_SIZE]);
        assert_eq!(inode.size(), BLOCK_SIZE);

        // No fsync on this inode; the only flush is the filesystem-level sync.
        f.ext4.sync_all().unwrap();

        let raw = f.read_raw_inode(11);
        assert_eq!(raw.size_lo, BLOCK_SIZE as u32);
        assert_eq!(raw.sector_count as u64, SECTORS_PER_BLOCK);
    }

    /// The corruption fix: a truncate on inode B that frees blocks must leave the
    /// on-disk inode and the block bitmap mutually consistent after `sync_all`.
    ///
    /// File A is written and fsync'd (its blocks persist). Then a *separate*
    /// inode B is truncated, freeing trailing blocks — dirtying the global bitmap
    /// and B's in-memory inode but persisting neither yet. `fs.sync_all()` (the
    /// clean-unmount path) must flush B's trimmed inode together with the bitmap:
    /// on disk, B's size reflects the truncate, every block B's extents still
    /// reference is allocated, and the freed trailing block is marked free.
    #[ktest]
    fn cross_inode_truncate_consistent_after_sync_all() {
        crate::time::clocks::init_for_ktest();
        let f = Ext4FixtureBuilder::new(2048, 256, 2048)
            .with_block_bitmap_metadata_marked()
            .build()
            .unwrap();
        f.write_raw_inode(11, &make_empty_file_inode()); // file A
        f.write_raw_inode(12, &make_empty_file_inode()); // file B

        // File A: write one block and fsync it (allocations persist to disk).
        let a = f.ext4.read_inode(11).unwrap();
        write_all(&a, 0, &[0xAA; BLOCK_SIZE]);
        a.sync_data_and_meta().unwrap();

        // File B: a 3-block file, then truncate to 1 block, freeing 2 trailing
        // blocks. The truncate updates the in-memory inode + the global bitmap
        // but does not, on its own, write B's inode back to disk.
        let b = f.ext4.read_inode(12).unwrap();
        write_all(&b, 0, &[0xBB; 3 * BLOCK_SIZE]);
        b.sync_data_and_meta().unwrap(); // B's 3 blocks are on disk and allocated

        let b2_pblock = ondisk_pblock_of(&f.read_raw_inode(12), 2).unwrap();
        assert!(block_is_allocated(&f, b2_pblock));

        b.resize(BLOCK_SIZE).unwrap();
        assert_eq!(b.size(), BLOCK_SIZE);

        // The clean-unmount path: flush all inodes + the block-side metadata.
        f.ext4.sync_all().unwrap();

        // B's on-disk inode reflects the truncate.
        let raw_b = f.read_raw_inode(12);
        assert_eq!(raw_b.size_lo, BLOCK_SIZE as u32);
        assert_eq!(raw_b.sector_count as u64, SECTORS_PER_BLOCK);

        // Consistency: every block B's on-disk extents still reference is marked
        // allocated, and the freed trailing block is now free in the bitmap.
        let b0_pblock = ondisk_pblock_of(&raw_b, 0).unwrap();
        assert!(block_is_allocated(&f, b0_pblock));
        assert!(ondisk_pblock_of(&raw_b, 2).is_none());
        assert!(!block_is_allocated(&f, b2_pblock));

        // A is untouched: its single block is still mapped and allocated.
        let a0_pblock = ondisk_pblock_of(&f.read_raw_inode(11), 0).unwrap();
        assert!(block_is_allocated(&f, a0_pblock));
    }

    /// alloc_ino then free_inode restores the group and superblock free-inode
    /// counters exactly.
    #[ktest]
    fn alloc_ino_free_inode_round_trip() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048)
            .with_inode_bitmap_metadata_marked()
            .build()
            .unwrap();

        let before_sb = f.ext4.super_block().free_inodes_count();
        let before_group = f.ext4.block_group(0).free_inodes_count();

        let ino = f.ext4.alloc_ino(ROOT_INO, InodeType::File).unwrap();
        assert_eq!(ino, 11); // first free inode after the 10 reserved ones
        assert_eq!(f.ext4.super_block().free_inodes_count(), before_sb - 1);
        assert_eq!(f.ext4.block_group(0).free_inodes_count(), before_group - 1);

        f.ext4.free_inode(ino, InodeType::File).unwrap();
        assert_eq!(f.ext4.super_block().free_inodes_count(), before_sb);
        assert_eq!(f.ext4.block_group(0).free_inodes_count(), before_group);
    }

    /// A full first group rings the allocation into the next group.
    #[ktest]
    fn alloc_ino_rings_to_next_group() {
        // A 2-group image with the normal reserved layout. Group 0 has 32 inodes
        // (10 reserved -> 22 free); exhaust them so the next allocation, with its
        // parent in group 0, must ring into group 1.
        let f = Ext4FixtureBuilder::new(256, 32, 2 * 256)
            .with_inode_bitmap_metadata_marked()
            .build()
            .unwrap();

        // Group 0 has 32 inodes, 10 reserved -> 22 free. Allocate all 22 so the
        // next allocation must ring into group 1.
        for _ in 0..22 {
            let ino = f.ext4.alloc_ino(ROOT_INO, InodeType::File).unwrap();
            assert!(ino <= 32, "ino {} should land in group 0", ino);
        }
        assert_eq!(f.ext4.block_group(0).free_inodes_count(), 0);

        // The 23rd allocation, with parent in group 0, rings to group 1.
        let ino = f.ext4.alloc_ino(ROOT_INO, InodeType::File).unwrap();
        assert!(ino > 32, "ino {} should ring into group 1", ino);
        assert_eq!(((ino - 1) / 32) as usize, 1);
    }

    /// A directory allocation bumps `used_dirs_count`; freeing it drops it back.
    #[ktest]
    fn alloc_dir_tracks_used_dirs_count() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048)
            .with_inode_bitmap_metadata_marked()
            .build()
            .unwrap();

        let before = f.ext4.block_group(0).used_dirs_count();
        let ino = f.ext4.alloc_ino(ROOT_INO, InodeType::Dir).unwrap();
        assert_eq!(f.ext4.block_group(0).used_dirs_count(), before + 1);

        f.ext4.free_inode(ino, InodeType::Dir).unwrap();
        assert_eq!(f.ext4.block_group(0).used_dirs_count(), before);
    }

    /// create_inode rolls back the inode allocation when the on-disk writeback
    /// fails: the bitmap bit is cleared and the superblock counter restored.
    #[ktest]
    fn create_inode_rolls_back_on_write_failure() {
        crate::time::clocks::init_for_ktest();
        let f = Ext4FixtureBuilder::new(2048, 256, 2048)
            .with_inode_bitmap_metadata_marked()
            .build()
            .unwrap();

        let before_sb = f.ext4.super_block().free_inodes_count();
        let before_group = f.ext4.block_group(0).free_inodes_count();

        // Force the inode writeback to fail, so create_inode must roll back.
        f.disk.set_fail_writes(true);
        let perm = FilePerm::from_bits_truncate(0o644);
        // `Arc<Inode>` is not `Debug`, so match instead of `unwrap_err`.
        let err = match f.ext4.create_inode(ROOT_INO, InodeType::File, perm) {
            Ok(_) => panic!("create_inode unexpectedly succeeded despite write failure"),
            Err(err) => err,
        };
        assert_eq!(err.error(), Errno::EIO);
        f.disk.set_fail_writes(false);

        // Allocation fully rolled back: counters restored and the freshly taken
        // bit (group-local index 10, i.e. ino 11) is clear again.
        assert_eq!(f.ext4.super_block().free_inodes_count(), before_sb);
        assert_eq!(f.ext4.block_group(0).free_inodes_count(), before_group);
        let group = f.ext4.block_group(0);
        assert!(!group.metadata().inode_bitmap.is_allocated(10));
    }

    /// On a 128-byte-inode volume, the absent extra area must decode as
    /// zeros (second-precision timestamps, default `crtime`) even when the
    /// bytes right after the slot -- the next inode -- are non-zero.
    #[ktest]
    fn slot_read_zero_fills_extra_area_on_128b_volume() {
        use super::super::test_utils::make_indirect_file_inode;

        let f = Ext4FixtureBuilder::new(2048, 256, 2048)
            .inode_size(128)
            .build()
            .unwrap();

        let mut raw = make_indirect_file_inode(100, 42);
        raw.atime = 1234;
        f.write_raw_inode(20, &raw);
        // Fill the next slot (bytes 128..256 after inode 20's start) with a
        // pattern a fixed 256-byte read would misdecode as the extra area.
        let neighbor_offset = 4 * BLOCK_SIZE + 20 * 128;
        f.disk
            .segment()
            .write_bytes(neighbor_offset, &[0xFF; 128])
            .unwrap();

        let desc = f.ext4.block_group(0).read_inode_desc(20).unwrap();
        assert_eq!(desc.atime().as_secs(), 1234);
        assert_eq!(desc.atime().subsec_nanos(), 0);
        assert_eq!(desc.crtime(), Duration::ZERO);
    }

    /// On a 256-byte-inode volume the slot RMW must keep preserving the
    /// unmodeled tail bytes exactly as the fixed-size RMW did.
    #[ktest]
    fn slot_write_back_preserves_tail_on_256b_volume() {
        use super::super::test_utils::make_indirect_file_inode;

        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();

        let mut raw = make_indirect_file_inode(100, 42);
        raw.tail.0[0] = 0xAB;
        raw.tail.0[7] = 0xCD;
        f.write_raw_inode(20, &raw);

        let inode = f.ext4.read_inode(20).unwrap();
        inode.set_mtime(Duration::from_secs(7777));
        inode.sync_metadata().unwrap();

        let after = f.read_raw_inode(20);
        assert_eq!(after.tail.0[0], 0xAB);
        assert_eq!(after.tail.0[7], 0xCD);
        assert_eq!(after.mtime, 7777);
    }

    /// The mount-flavor decision matrix: an `ext2` mount accepts only true
    /// ext2-format volumes (Linux's `IS_EXT2_SB` rule); extent-mapped or
    /// journaled volumes must be mounted as `ext4` instead. The `ext4` name
    /// accepts both formats (a clean journal is ignored). The historic
    /// `btree_dir` read-only-compat bit stays accepted under either name.
    #[ktest]
    fn mount_flavor_decision_matrix() {
        // An ext2-format volume (no EXTENTS): both names accept it.
        Ext4FixtureBuilder::new(2048, 256, 2048)
            .without_extents_feature()
            .build()
            .unwrap();
        Ext4FixtureBuilder::new(2048, 256, 2048)
            .without_extents_feature()
            .with_flavor(MountFlavor::Ext2)
            .build()
            .unwrap();

        // An extent volume is rejected under the ext2 name.
        let err = match Ext4FixtureBuilder::new(2048, 256, 2048)
            .with_flavor(MountFlavor::Ext2)
            .build()
        {
            Err(err) => err,
            Ok(_) => panic!("an extent volume must not mount as ext2"),
        };
        assert_eq!(err.error(), Errno::EINVAL);

        // A journaled volume mounts as ext4 (journal ignored) but is
        // rejected under the ext2 name.
        Ext4FixtureBuilder::new(2048, 256, 2048)
            .with_extra_compat(0x4) // COMPAT_HAS_JOURNAL
            .build()
            .unwrap();
        let err = match Ext4FixtureBuilder::new(2048, 256, 2048)
            .without_extents_feature()
            .with_extra_compat(0x4)
            .with_flavor(MountFlavor::Ext2)
            .build()
        {
            Err(err) => err,
            Ok(_) => panic!("a journaled volume must not mount as ext2"),
        };
        assert_eq!(err.error(), Errno::EINVAL);

        // The historic BTREE_DIR read-only-compat bit predates the checksum
        // features and stays accepted under either name (ext2-driver parity).
        Ext4FixtureBuilder::new(2048, 256, 2048)
            .with_extra_ro_compat(0x4) // RO_COMPAT_BTREE_DIR
            .build()
            .unwrap();
    }

    /// An ext2-format volume (no EXTENTS feature) mounts, creates ext2-format
    /// inodes (DR-9), and serves buffered write/read/shrink through the
    /// indirect engine end to end.
    #[ktest]
    fn ext2_format_volume_end_to_end() {
        use super::super::inode::EXTENTS_FL;

        crate::time::clocks::init_for_ktest();
        let f = Ext4FixtureBuilder::new(2048, 256, 2048)
            .without_extents_feature()
            .with_block_bitmap_metadata_marked()
            .with_inode_bitmap_metadata_marked()
            .build()
            .unwrap();

        let inode = f
            .ext4
            .create_inode(
                ROOT_INO,
                InodeType::File,
                FilePerm::from_bits_truncate(0o644),
            )
            .unwrap();
        let ino = inode.ino();

        // The new inode is pure ext2 format on disk (`create_inode` writes the
        // fresh slot immediately): no `EXTENTS` flag and an all-zero pointer
        // array, so an ext2 driver could still mount the volume.
        let raw = f.read_raw_inode(ino);
        assert_eq!(raw.flags & EXTENTS_FL, 0);
        assert_eq!(raw.block, [0u32; super::super::inode::RAW_BLOCK_PTRS_LEN]);

        // Buffered write and read back through the indirect engine.
        let data: Vec<u8> = (0..BLOCK_SIZE * 3 + 123).map(|i| i as u8).collect();
        let mut reader = VmReader::from(&data[..]).to_fallible();
        assert_eq!(inode.write_at(0, &mut reader).unwrap(), data.len());
        let mut buf = vec![0u8; data.len()];
        let mut writer = VmWriter::from(buf.as_mut_slice()).to_fallible();
        assert_eq!(inode.read_at(0, &mut writer).unwrap(), data.len());
        assert_eq!(buf, data);

        // Shrinking frees the tail blocks and persists direct pointers.
        inode.resize(BLOCK_SIZE).unwrap();
        inode.sync_metadata().unwrap();
        let raw = f.read_raw_inode(ino);
        assert_ne!(raw.block[0], 0);
        assert_eq!(raw.block[1], 0);
    }

    fn expected_overhead_blocks(fs: &Ext4) -> u64 {
        let sb = fs.super_block();
        let nr_block_groups = sb.nr_block_groups() as usize;
        let gdb_count =
            ((nr_block_groups * size_of::<RawBlockGroup>()).div_ceil(BLOCK_SIZE)) as u64;
        let mut overhead = sb.first_data_block();

        for group_idx in 0..nr_block_groups {
            if group_idx == 0 || sb.is_backup_group(group_idx) {
                overhead += 1 + gdb_count;
            }
        }

        overhead
            + u64::from(sb.nr_block_groups())
                * (2 + u64::from(sb.nr_inode_table_blocks_per_group()))
    }

    /// The default (`bsddf`) accounting subtracts metadata overhead from the
    /// reported total block count.
    #[ktest]
    fn stat_bsddf_subtracts_overhead() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();

        let stat = FileSystemTrait::sb(f.ext4.as_ref());
        let expected_overhead = expected_overhead_blocks(&f.ext4);

        assert_eq!(
            stat.blocks,
            (f.ext4.super_block().total_blocks() - expected_overhead) as usize
        );
        assert!(stat.blocks < f.ext4.super_block().total_blocks() as usize);
    }

    /// The `minixdf` mount option reports the raw total block count.
    #[ktest]
    fn stat_minixdf_reports_total() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        let minixdf = CString::new("minixdf").unwrap();

        let ext4 = Ext4::open(
            f.disk.clone() as Arc<dyn BlockDevice>,
            MountFlavor::Ext4,
            Some(minixdf.as_c_str()),
        )
        .unwrap();

        let stat = FileSystemTrait::sb(ext4.as_ref());
        assert_eq!(stat.blocks, f.ext4.super_block().total_blocks() as usize);
    }

    /// `sync_metadata` stamps the primary superblock's write time and leaves
    /// backup superblock copies untouched: the running filesystem maintains
    /// only the primary; backups belong to mkfs/fsck/resize (matching Linux).
    #[ktest]
    fn sync_stamps_wtime_and_leaves_backups_alone() {
        crate::time::clocks::init_for_ktest();
        let f = Ext4FixtureBuilder::new(1024, 256, 2048)
            .with_block_bitmap_metadata_marked()
            .build()
            .unwrap();

        // Plant a sentinel write time on disk; the sync below must overwrite
        // it with the current time (the ktest clock reads as the epoch, so
        // "was patched" is asserted as "no longer the sentinel").
        let sentinel = Duration::from_secs(12345);
        let mut raw = f
            .disk
            .segment()
            .read_val::<RawSuperBlock>(SUPER_BLOCK_OFFSET)
            .unwrap();
        raw.wtime = sentinel.into();
        f.disk
            .segment()
            .write_val(SUPER_BLOCK_OFFSET, &raw)
            .unwrap();

        // Snapshot the group-1 backup-superblock area before the sync.
        let backup_offset = (f.ext4.super_block().group_first_block_no(1) as usize) * BLOCK_SIZE;
        let mut before = vec![0u8; 1024];
        f.disk
            .segment()
            .read_bytes(backup_offset, &mut before)
            .unwrap();

        // Dirty the superblock (allocation moves the free-block counter).
        let range = f.ext4.alloc_blocks(1, 0).unwrap();
        assert!(!range.is_empty());
        f.ext4.sync_metadata().unwrap();

        let raw = f
            .disk
            .segment()
            .read_val::<RawSuperBlock>(SUPER_BLOCK_OFFSET)
            .unwrap();
        assert_ne!(Duration::from(raw.wtime), sentinel);

        let mut after = vec![0u8; 1024];
        f.disk
            .segment()
            .read_bytes(backup_offset, &mut after)
            .unwrap();
        assert_eq!(before, after);
    }
}
