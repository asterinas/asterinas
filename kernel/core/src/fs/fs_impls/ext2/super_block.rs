// SPDX-License-Identifier: MPL-2.0

//! Ext2 superblock: on-disk layout, in-memory representation, and mount validation.
//!
//! The superblock is the filesystem-wide metadata record that describes the
//! overall layout and global state of an ext2 volume. It is stored at a
//! fixed offset of 1024 bytes from the start of the device (`SUPER_BLOCK_OFFSET`).
//!
//! # Types
//!
//! - `RawSuperBlock` — the `#[repr(C)]` on-disk layout.
//! - `SuperBlock` — a validated, in-memory representation.
//!
//! # Mount validation
//!
//! `SuperBlock::try_from` enforces the subset of ext2 constraints that
//! Asterinas currently supports:
//! - Magic number must be `0xEF53`.
//! - Block size must be 4096 bytes (log₂ shift = 2).
//! - Fragment size must equal block size.
//! - Creator OS must be Linux.
//! - Error behavior must be `Continue`.
//! - Incompatible and read-only compatible feature sets are checked against
//!   the supported masks. Unknown incompatible or read-only compatible
//!   features cause mount failure; compatible features are retained only when
//!   represented by the known bitflags.
//!
//! # Superblock copies
//!
//! Ext2 stores backup superblock copies in selected block groups. Asterinas
//! reads the primary copy during mount and writes both the primary copy and
//! all supported backup copies during metadata sync.

use ostd::const_assert;

use super::{block_group::RawBlockGroup, prelude::*};
use crate::fs::ext2::utils;

/// The ext2 magic number.
pub(super) const MAGIC_NUM: u16 = 0xef53;

/// The main superblock is located at byte 1024 from the beginning of the device.
pub(super) const SUPER_BLOCK_OFFSET: usize = 1024;

const SUPER_BLOCK_SIZE: usize = 1024;

/// Validated, Rust-typed in-memory representation of the ext2 superblock.
#[derive(Clone, Copy, Debug)]
pub(super) struct SuperBlock {
    /// Total number of inodes.
    inodes_count: u32,
    /// Total number of blocks.
    blocks_count: u32,
    /// Total number of reserved blocks.
    reserved_blocks_count: u32,
    /// Total number of free blocks.
    free_blocks_count: u32,
    /// Total number of free inodes.
    free_inodes_count: u32,
    /// First data block.
    first_data_block: Ext2Bid,
    /// Block size.
    block_size: usize,
    /// Fragment size.
    frag_size: usize,
    /// Number of blocks in each block group.
    nr_blocks_per_group: u32,
    /// Number of fragments in each block group.
    nr_frags_per_group: u32,
    /// Number of inodes in each block group.
    nr_inodes_per_group: u32,
    /// Number of inode table blocks in each group.
    nr_inode_table_blocks_per_group: u32,
    /// Mount time.
    mtime: Duration,
    /// Write time.
    wtime: Duration,
    /// Mount count.
    mnt_count: u16,
    /// Maximal mount count.
    max_mnt_count: u16,
    /// Magic signature.
    magic: u16,
    /// File system state.
    state: FsState,
    /// Behavior when detecting errors.
    errors_behavior: ErrorsBehavior,
    /// Time of last check.
    last_check_time: Duration,
    /// Interval between checks.
    check_interval: Duration,
    /// Creator OS ID.
    creator_os: OsId,
    /// Revision level.
    rev_level: RevLevel,
    /// Default UID for reserved blocks.
    default_reserved_uid: u32,
    /// Default GID for reserved blocks.
    default_reserved_gid: u32,

    // These fields are valid for RevLevel::Dynamic only.
    /// First non-reserved inode number.
    first_ino: u32,
    /// Size of inode structure.
    inode_size: usize,
    /// Block group that this superblock is part of (if backup copy).
    block_group_idx: usize,
    /// Compatible feature set.
    feature_compat: FeatureCompatSet,
    /// Incompatible feature set.
    feature_incompat: FeatureInCompatSet,
    /// Read-only-compatible feature set.
    feature_ro_compat: FeatureRoCompatSet,
    /// 128-bit UUID for the volume.
    uuid: [u8; 16],
    /// Volume name.
    volume_name: Str16,
    /// Directory where last mounted.
    last_mounted_dir: Str64,

    // These fields are valid if the FeatureCompatSet::DIR_PREALLOC is set.
    /// Number of blocks to preallocate for files.
    prealloc_file_blocks: u8,
    /// Number of blocks to preallocate for directories.
    prealloc_dir_blocks: u8,
    //
    // These fields are reserved and currently serve no purpose.
    min_rev_level: u16,
    algorithm_usage_bitmap: u32,
    padding1: u16,
    journal_uuid: [u8; 16],
    journal_ino: u32,
    journal_dev: u32,
    last_orphan: u32,
    hash_seed: [u32; 4],
    def_hash_version: u8,
    reserved_char_pad: u8,
    reserved_word_pad: u16,
    default_mount_opts: u32,
    first_meta_bg: u32,
    reserved: Reserved,
}

impl TryFrom<RawSuperBlock> for SuperBlock {
    type Error = Error;

    fn try_from(sb: RawSuperBlock) -> Result<Self> {
        if sb.magic != MAGIC_NUM {
            return_errno_with_message!(Errno::EINVAL, "bad ext2 magic number");
        }

        if sb.log_block_size != 2 {
            return_errno_with_message!(Errno::EINVAL, "unsupported block size");
        }
        if sb.log_frag_size != sb.log_block_size {
            return_errno_with_message!(Errno::EINVAL, "invalid fragment size");
        }

        let block_size = BLOCK_SIZE;
        let frag_size = BLOCK_SIZE;

        let state = FsState::from_bits(sb.state)
            .ok_or(Error::with_message(Errno::EINVAL, "invalid fs state"))?;

        let errors_behavior = ErrorsBehavior::try_from(sb.errors)
            .map_err(|_| Error::with_message(Errno::EINVAL, "invalid errors behavior"))?;
        if errors_behavior != ErrorsBehavior::Continue {
            return_errno_with_message!(Errno::EINVAL, "unsupported errors behavior");
        }

        let creator_os = OsId::try_from(sb.creator_os)
            .map_err(|_| Error::with_message(Errno::EINVAL, "invalid creator os"))?;
        if creator_os != OsId::Linux {
            return_errno_with_message!(Errno::EINVAL, "not supported os id");
        }

        let rev_level = RevLevel::try_from(sb.rev_level)
            .map_err(|_| Error::with_message(Errno::EINVAL, "invalid revision level"))?;
        let (first_ino, inode_size) = match rev_level {
            RevLevel::GoodOld => (11, 128usize),
            RevLevel::Dynamic => {
                let inode_size = sb.inode_size as usize;
                if inode_size < 128 {
                    return_errno_with_message!(Errno::EINVAL, "inode size is too small");
                }
                if inode_size > BLOCK_SIZE {
                    return_errno_with_message!(Errno::EINVAL, "inode size is too large");
                }
                if !inode_size.is_power_of_two() {
                    return_errno_with_message!(Errno::EINVAL, "inode size is not power of two");
                }
                (sb.first_ino, inode_size)
            }
        };

        let nr_inodes_per_group = sb.inodes_per_group;
        let nr_blocks_per_group = sb.blocks_per_group;
        if nr_inodes_per_group == 0 || nr_blocks_per_group == 0 {
            return_errno_with_message!(Errno::EINVAL, "invalid group sizes");
        }

        let inodes_per_block = (block_size / inode_size) as u32;
        if nr_inodes_per_group < inodes_per_block {
            return_errno_with_message!(Errno::EINVAL, "inodes per group is too small");
        }

        let max_bits_per_group = (block_size as u32) * 8;
        if nr_inodes_per_group > max_bits_per_group {
            return_errno_with_message!(Errno::EINVAL, "inodes per group is too large");
        }
        if nr_blocks_per_group > max_bits_per_group {
            return_errno_with_message!(Errno::EINVAL, "blocks per group is too large");
        }

        let nr_inode_table_blocks_per_group = nr_inodes_per_group / inodes_per_block;
        if nr_blocks_per_group <= nr_inode_table_blocks_per_group + 3 {
            return_errno_with_message!(Errno::EINVAL, "blocks per group is too small");
        }

        let blocks_count = sb.blocks_count as u64;
        let first_data_block = sb.first_data_block as u64;
        if blocks_count <= first_data_block + 1 {
            return_errno_with_message!(Errno::EINVAL, "invalid blocks count");
        }
        let blocks_after_first_data = blocks_count - first_data_block - 1;
        let nr_block_groups = (blocks_after_first_data / nr_blocks_per_group as u64) + 1;

        // The last block group may legitimately contain fewer inodes than the
        // full per-group capacity.
        let max_inodes = nr_block_groups * (nr_inodes_per_group as u64);
        let min_inodes = (nr_block_groups - 1) * (nr_inodes_per_group as u64);
        let inodes_count = sb.inodes_count as u64;
        if inodes_count <= min_inodes || inodes_count > max_inodes {
            return_errno_with_message!(Errno::EINVAL, "invalid inodes count");
        }
        if sb.free_blocks_count > sb.blocks_count {
            return_errno_with_message!(Errno::EINVAL, "free blocks count exceeds blocks count");
        }
        if sb.free_inodes_count > sb.inodes_count {
            return_errno_with_message!(Errno::EINVAL, "free inodes count exceeds inodes count");
        }

        let feature_compat = FeatureCompatSet::from_bits_truncate(sb.feature_compat);

        let allowed_incompat = FeatureInCompatSet::FILETYPE.bits();
        if (sb.feature_incompat & !allowed_incompat) != 0 {
            return_errno_with_message!(Errno::EINVAL, "unsupported incompat feature");
        }
        let feature_incompat = FeatureInCompatSet::from_bits_truncate(sb.feature_incompat);

        let allowed_ro_compat = FeatureRoCompatSet::SPARSE_SUPER.bits()
            | FeatureRoCompatSet::LARGE_FILE.bits()
            | FeatureRoCompatSet::BTREE_DIR.bits();
        if (sb.feature_ro_compat & !allowed_ro_compat) != 0 {
            return_errno_with_message!(Errno::EINVAL, "unsupported ro-compatible feature");
        }
        let feature_ro_compat = FeatureRoCompatSet::from_bits_truncate(sb.feature_ro_compat);

        Ok(Self {
            inodes_count: sb.inodes_count,
            blocks_count: sb.blocks_count,
            reserved_blocks_count: sb.reserved_blocks_count,
            free_blocks_count: sb.free_blocks_count,
            free_inodes_count: sb.free_inodes_count,
            first_data_block: sb.first_data_block,
            block_size,
            frag_size,
            nr_blocks_per_group: sb.blocks_per_group,
            nr_frags_per_group: sb.frags_per_group,
            nr_inodes_per_group: sb.inodes_per_group,
            nr_inode_table_blocks_per_group,
            mtime: Duration::from(sb.mtime),
            wtime: Duration::from(sb.wtime),
            mnt_count: sb.mnt_count,
            max_mnt_count: sb.max_mnt_count,
            magic: MAGIC_NUM,
            state,
            errors_behavior,
            last_check_time: Duration::from(sb.last_check_time),
            check_interval: Duration::from_secs(sb.check_interval as _),
            creator_os,
            rev_level,
            default_reserved_uid: sb.default_reserved_uid as _,
            default_reserved_gid: sb.default_reserved_gid as _,
            first_ino,
            inode_size,
            block_group_idx: sb.block_group_idx as _,
            feature_compat,
            feature_incompat,
            feature_ro_compat,
            uuid: sb.uuid,
            volume_name: sb.volume_name,
            last_mounted_dir: sb.last_mounted_dir,
            prealloc_file_blocks: sb.prealloc_file_blocks,
            prealloc_dir_blocks: sb.prealloc_dir_blocks,
            min_rev_level: sb.min_rev_level,
            algorithm_usage_bitmap: sb.algorithm_usage_bitmap,
            padding1: sb.padding1,
            journal_uuid: sb.journal_uuid,
            journal_ino: sb.journal_ino,
            journal_dev: sb.journal_dev,
            last_orphan: sb.last_orphan,
            hash_seed: sb.hash_seed,
            def_hash_version: sb.def_hash_version,
            reserved_char_pad: sb.reserved_char_pad,
            reserved_word_pad: sb.reserved_word_pad,
            default_mount_opts: sb.default_mount_opts,
            first_meta_bg: sb.first_meta_bg,
            reserved: sb.reserved,
        })
    }
}

impl SuperBlock {
    /// Returns the block size.
    pub(super) const fn block_size(&self) -> usize {
        self.block_size
    }

    /// Returns the maximum regular file size supported by this ext2 instance.
    pub(super) fn max_file_size(&self) -> usize {
        let max_blocks = self.max_blocks();
        let block_size_bits = self.block_size.trailing_zeros();
        let max_bytes = (max_blocks << block_size_bits).min(i64::MAX as u64);
        max_bytes as usize
    }

    const fn max_blocks(&self) -> u64 {
        const DIRECT_BLOCKS: u64 = 12;

        let block_size_bits = self.block_size.trailing_zeros();
        let ptrs_per_block = 1u64 << (block_size_bits - 2);
        let mut max_blocks = DIRECT_BLOCKS;
        let mut metadata_blocks = 1u64;
        let mut upper_limit = (1u64 << 32) - 1;

        // `i_blocks` stores the total 512-byte sector count for data and
        // indirect metadata. Linux keeps ext2 one filesystem block below the
        // 32-bit sector accounting limit.
        upper_limit >>= block_size_bits - 9;

        max_blocks += 1u64 << (block_size_bits - 2);
        max_blocks += 1u64 << (2 * (block_size_bits - 2));
        max_blocks += 1u64 << (3 * (block_size_bits - 2));

        metadata_blocks += 1 + ptrs_per_block;
        metadata_blocks += 1 + ptrs_per_block + ptrs_per_block * ptrs_per_block;

        if max_blocks + metadata_blocks > upper_limit {
            max_blocks = upper_limit;

            upper_limit -= DIRECT_BLOCKS;

            metadata_blocks = 1;
            upper_limit -= ptrs_per_block;

            if upper_limit < ptrs_per_block * ptrs_per_block {
                metadata_blocks += 1 + upper_limit.div_ceil(ptrs_per_block);
                max_blocks -= metadata_blocks;
            } else {
                metadata_blocks += 1 + ptrs_per_block;
                upper_limit -= ptrs_per_block * ptrs_per_block;
                metadata_blocks += 1
                    + upper_limit.div_ceil(ptrs_per_block)
                    + upper_limit.div_ceil(ptrs_per_block * ptrs_per_block);
                max_blocks -= metadata_blocks;
            }
        }
        max_blocks
    }

    /// Returns the size of inode structure.
    pub(super) const fn inode_size(&self) -> usize {
        self.inode_size
    }

    /// Returns the fragment size.
    pub(super) const fn fragment_size(&self) -> usize {
        self.frag_size
    }

    /// Returns total number of inodes.
    pub(super) const fn total_inodes(&self) -> u32 {
        self.inodes_count
    }

    /// Returns total number of blocks.
    pub(super) const fn total_blocks(&self) -> u32 {
        self.blocks_count
    }

    /// Returns the number of blocks in each block group.
    pub(super) const fn nr_blocks_per_group(&self) -> u32 {
        self.nr_blocks_per_group
    }

    /// Returns the first block number of a block group.
    pub(super) const fn group_first_block_no(&self, group_idx: usize) -> u32 {
        (group_idx as u32) * self.nr_blocks_per_group + self.first_data_block()
    }

    /// Returns the last block number of a block group.
    pub(super) fn group_last_block_no(&self, group_idx: usize) -> u32 {
        let nr_block_groups = self.nr_block_groups();
        if group_idx as u32 == nr_block_groups - 1 {
            self.total_blocks() - 1
        } else {
            self.group_first_block_no(group_idx) + self.nr_blocks_per_group - 1
        }
    }

    /// Returns whether a data block range is valid.
    pub(super) fn is_data_block_valid(&self, start_blk: u32, count: u32) -> bool {
        if count == 0 {
            return false;
        }

        let first_data_block = self.first_data_block();
        let blocks_count = self.total_blocks();

        let Some(end_blk) = start_blk.checked_add(count - 1) else {
            return false;
        };

        if start_blk <= first_data_block || end_blk >= blocks_count {
            return false;
        }

        let sb_bid = if self.block_size == SUPER_BLOCK_SIZE {
            1u32
        } else {
            0u32
        };
        if start_blk <= sb_bid && end_blk >= sb_bid {
            return false;
        }

        true
    }

    /// Returns the first data block number.
    pub(super) const fn first_data_block(&self) -> Ext2Bid {
        self.first_data_block
    }

    /// Returns the number of inodes in each block group.
    pub(super) const fn nr_inodes_per_group(&self) -> u32 {
        self.nr_inodes_per_group
    }

    /// Returns the number of inode table blocks in each block group.
    pub(super) const fn nr_inode_table_blocks_per_group(&self) -> u32 {
        self.nr_inode_table_blocks_per_group
    }

    /// Returns the first non-reserved inode number.
    pub(super) const fn first_ino(&self) -> u32 {
        self.first_ino
    }

    /// Returns the number of block groups.
    pub(super) const fn nr_block_groups(&self) -> u32 {
        self.blocks_count.div_ceil(self.nr_blocks_per_group)
    }

    /// Returns the number of group descriptor blocks in each superblock copy.
    pub(super) const fn group_descriptor_blocks_count(&self) -> u32 {
        let group_desc_bytes = (self.nr_block_groups() as usize) * size_of::<RawBlockGroup>();
        group_desc_bytes.div_ceil(self.block_size) as u32
    }

    /// Returns the number of free blocks.
    pub(super) const fn free_blocks_count(&self) -> u32 {
        self.free_blocks_count
    }

    /// Returns the number of reserved blocks.
    pub(super) const fn reserved_blocks_count(&self) -> u32 {
        self.reserved_blocks_count
    }

    /// Returns the default UID for reserved blocks.
    pub(super) const fn default_reserved_uid(&self) -> u32 {
        self.default_reserved_uid
    }

    /// Returns the default GID for reserved blocks.
    pub(super) const fn default_reserved_gid(&self) -> u32 {
        self.default_reserved_gid
    }

    /// Increases the number of free blocks.
    pub(super) fn inc_free_blocks(&mut self, count: u32) -> Result<()> {
        self.free_blocks_count = self
            .free_blocks_count
            .checked_add(count)
            .ok_or_else(|| Error::with_message(Errno::EIO, "free block counter overflow"))?;
        Ok(())
    }

    /// Overwrites the free blocks counter with a recomputed value.
    pub(super) const fn set_free_blocks_count(&mut self, count: u32) {
        self.free_blocks_count = count;
    }

    /// Decreases the number of free blocks.
    pub(super) fn dec_free_blocks(&mut self, count: u32) -> Result<()> {
        self.free_blocks_count = self
            .free_blocks_count
            .checked_sub(count)
            .ok_or_else(|| Error::with_message(Errno::EIO, "free block counter underflow"))?;
        Ok(())
    }

    /// Returns the number of free inodes.
    pub(super) const fn free_inodes_count(&self) -> u32 {
        self.free_inodes_count
    }

    /// Overwrites the free inodes counter with a recomputed value.
    pub(super) const fn set_free_inodes_count(&mut self, count: u32) {
        self.free_inodes_count = count;
    }

    /// Increases the number of free inodes.
    pub(super) fn inc_free_inodes(&mut self) -> Result<()> {
        self.free_inodes_count = self
            .free_inodes_count
            .checked_add(1)
            .ok_or_else(|| Error::with_message(Errno::EIO, "free inode counter overflow"))?;
        Ok(())
    }

    /// Sets the last write time.
    pub(super) const fn set_wtime(&mut self, time: Duration) {
        self.wtime = time;
    }

    /// Decreases the number of free inodes.
    pub(super) fn dec_free_inodes(&mut self) -> Result<()> {
        self.free_inodes_count = self
            .free_inodes_count
            .checked_sub(1)
            .ok_or_else(|| Error::with_message(Errno::EIO, "free inode counter underflow"))?;
        Ok(())
    }

    /// Checks if the block group will backup the super block.
    pub(super) fn is_backup_group(&self, block_group_idx: usize) -> bool {
        if block_group_idx == 0 {
            false
        } else if self
            .feature_ro_compat
            .contains(FeatureRoCompatSet::SPARSE_SUPER)
        {
            // The backup groups chosen are 1 and powers of 3, 5 and 7.
            block_group_idx == 1
                || block_group_idx.is_power_of(3)
                || block_group_idx.is_power_of(5)
                || block_group_idx.is_power_of(7)
        } else {
            true
        }
    }

    /// Returns whether the given group stores the primary or a backup superblock.
    pub(super) fn has_super_block(&self, block_group_idx: usize) -> bool {
        block_group_idx == 0 || self.is_backup_group(block_group_idx)
    }

    /// Computes the metadata overhead subtracted from reported block totals.
    pub(super) fn total_metadata_blocks(&self) -> u32 {
        let nr_block_groups = self.nr_block_groups() as usize;
        let group_desc_blocks_count = self.group_descriptor_blocks_count();
        let mut overhead = self.first_data_block();

        for group_idx in 0..nr_block_groups {
            if self.has_super_block(group_idx) {
                overhead = overhead.saturating_add(1 + group_desc_blocks_count);
            }
        }

        overhead.saturating_add(
            self.nr_block_groups()
                .saturating_mul(2u32.saturating_add(self.nr_inode_table_blocks_per_group)),
        )
    }

    /// Returns the starting block ID of the superblock copy inside the block
    /// group identified by `block_group_idx`.
    ///
    pub(super) fn bid(&self, block_group_idx: usize) -> Ext2Bid {
        if block_group_idx == 0 {
            return (SUPER_BLOCK_OFFSET / self.block_size) as u32;
        }
        debug_assert!(self.is_backup_group(block_group_idx));
        block_group_idx as u32 * self.nr_blocks_per_group + self.first_data_block()
    }

    /// Returns the starting block ID of the block-group descriptor table inside
    /// the block group identified by `block_group_idx`.
    pub(super) fn group_descriptors_bid(&self, block_group_idx: usize) -> Ext2Bid {
        let sb_bid = self.bid(block_group_idx);
        sb_bid + (SUPER_BLOCK_SIZE.div_ceil(self.block_size) as u32)
    }

    #[expect(dead_code)]
    const fn state(&self) -> FsState {
        self.state
    }

    #[expect(dead_code)]
    const fn rev_level(&self) -> RevLevel {
        self.rev_level
    }

    #[expect(dead_code)]
    const fn feature_compat(&self) -> FeatureCompatSet {
        self.feature_compat
    }

    #[expect(dead_code)]
    const fn feature_incompat(&self) -> FeatureInCompatSet {
        self.feature_incompat
    }

    #[expect(dead_code)]
    const fn feature_ro_compat(&self) -> FeatureRoCompatSet {
        self.feature_ro_compat
    }
}

bitflags! {
    /// Compatible feature set.
    struct FeatureCompatSet: u32 {
        /// Preallocate some number of blocks to a directory when creating a new one.
        const DIR_PREALLOC = 1 << 0;
        /// AFS server inodes exist.
        const IMAGIC_INODES = 1 << 1;
        /// File system has a journal.
        const HAS_JOURNAL = 1 << 2;
        /// Inodes have extended attributes.
        const EXT_ATTR = 1 << 3;
        /// File system can resize itself for larger partitions.
        const RESIZE_INO = 1 << 4;
        /// Directories use hash index.
        const DIR_INDEX = 1 << 5;
    }
}

bitflags! {
    /// Incompatible feature set.
    struct FeatureInCompatSet: u32 {
        /// Compression is used.
        const COMPRESSION = 1 << 0;
        /// Directory entries contain a type field.
        const FILETYPE = 1 << 1;
        /// File system needs to replay its journal.
        const RECOVER = 1 << 2;
        /// File system uses a journal device.
        const JOURNAL_DEV = 1 << 3;
        /// Metablock block group.
        const META_BG = 1 << 4;
    }
}

bitflags! {
    /// Read-only-compatible feature set.
    struct FeatureRoCompatSet: u32 {
        /// Sparse superblocks and group descriptor tables.
        const SPARSE_SUPER = 1 << 0;
        /// File system uses a 64-bit file size.
        const LARGE_FILE = 1 << 1;
        /// Directory contents are stored in a binary tree.
        const BTREE_DIR = 1 << 2;
    }
}

bitflags! {
    /// File system state.
    ///
    /// Reference: <https://www.nongnu.org/ext2-doc/ext2.html#s-state>
    pub(super) struct FsState: u16 {
        /// Unmounted cleanly.
        const VALID = 1 << 0;
        /// Errors detected.
        const ERROR = 1 << 1;
    }
}

/// Action the filesystem driver takes when an error is detected.
#[repr(u16)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, TryFromInt)]
pub(super) enum ErrorsBehavior {
    /// Continues execution.
    #[default]
    Continue = 1,
    /// Remounts the filesystem read-only.
    RemountReadonly = 2,
    /// Panics.
    Panic = 3,
}

/// OS that created the filesystem (`s_creator_os`).
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub(super) enum OsId {
    Linux = 0,
    Hurd = 1,
    Masix = 2,
    FreeBSD = 3,
    Lites = 4,
}

/// The ext2 revision level (`s_rev_level`).
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub(super) enum RevLevel {
    /// The good old (original) format.
    GoodOld = 0,
    /// V2 format with dynamic inode size.
    Dynamic = 1,
}

const_assert!(size_of::<RawSuperBlock>() == SUPER_BLOCK_SIZE);

/// The on-disk superblock structure.
///
/// Must be exactly 1024 bytes to match the ext2 specification. Convert to
/// `SuperBlock` via `TryFrom` for the validated in-memory representation.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct RawSuperBlock {
    pub inodes_count: u32,
    pub blocks_count: u32,
    pub reserved_blocks_count: u32,
    pub free_blocks_count: u32,
    pub free_inodes_count: u32,
    pub first_data_block: u32,
    /// The number to left-shift 1024 to obtain the block size.
    pub log_block_size: u32,
    /// The number to left-shift 1024 to obtain the fragment size.
    pub log_frag_size: u32,
    pub blocks_per_group: u32,
    pub frags_per_group: u32,
    pub inodes_per_group: u32,
    /// Mount time.
    pub mtime: UnixTime,
    /// Write time.
    pub wtime: UnixTime,
    pub mnt_count: u16,
    pub max_mnt_count: u16,
    pub magic: u16,
    pub state: u16,
    pub errors: u16,
    pub min_rev_level: u16,
    /// Time of last check.
    pub last_check_time: UnixTime,
    pub check_interval: u32,
    pub creator_os: u32,
    pub rev_level: u32,
    pub default_reserved_uid: u16,
    pub default_reserved_gid: u16,
    pub first_ino: u32,
    pub inode_size: u16,
    pub block_group_idx: u16,
    pub feature_compat: u32,
    pub feature_incompat: u32,
    pub feature_ro_compat: u32,
    pub uuid: [u8; 16],
    pub volume_name: Str16,
    pub last_mounted_dir: Str64,
    pub algorithm_usage_bitmap: u32,
    pub prealloc_file_blocks: u8,
    pub prealloc_dir_blocks: u8,
    padding1: u16,
    // These fields are for journaling support in Ext3.
    /// UUID of the journal superblock.
    pub journal_uuid: [u8; 16],
    /// Inode number of journal file.
    pub journal_ino: u32,
    /// Device number of journal file.
    pub journal_dev: u32,
    /// Start of list of inodes to delete.
    pub last_orphan: u32,
    /// HTREE hash seed.
    pub hash_seed: [u32; 4],
    /// Default hash version to use.
    pub def_hash_version: u8,
    reserved_char_pad: u8,
    reserved_word_pad: u16,
    /// Default mount options.
    pub default_mount_opts: u32,
    /// First metablock block group.
    pub first_meta_bg: u32,
    reserved: Reserved,
}

impl From<&SuperBlock> for RawSuperBlock {
    fn from(sb: &SuperBlock) -> Self {
        Self {
            inodes_count: sb.inodes_count,
            blocks_count: sb.blocks_count,
            reserved_blocks_count: sb.reserved_blocks_count,
            free_blocks_count: sb.free_blocks_count,
            free_inodes_count: sb.free_inodes_count,
            first_data_block: sb.first_data_block,
            log_block_size: (sb.block_size / SUPER_BLOCK_SIZE).trailing_zeros(),
            log_frag_size: (sb.frag_size / SUPER_BLOCK_SIZE).trailing_zeros(),
            blocks_per_group: sb.nr_blocks_per_group,
            frags_per_group: sb.nr_frags_per_group,
            inodes_per_group: sb.nr_inodes_per_group,
            mtime: UnixTime::from(sb.mtime),
            wtime: UnixTime::from(sb.wtime),
            mnt_count: sb.mnt_count,
            max_mnt_count: sb.max_mnt_count,
            magic: sb.magic,
            state: sb.state.bits(),
            errors: sb.errors_behavior as u16,
            min_rev_level: sb.min_rev_level,
            last_check_time: UnixTime::from(sb.last_check_time),
            check_interval: utils::duration_to_ext2_secs(sb.check_interval),
            creator_os: sb.creator_os as u32,
            rev_level: sb.rev_level as u32,
            default_reserved_uid: sb.default_reserved_uid as u16,
            default_reserved_gid: sb.default_reserved_gid as u16,
            first_ino: sb.first_ino,
            inode_size: sb.inode_size as u16,
            block_group_idx: sb.block_group_idx as u16,
            feature_compat: sb.feature_compat.bits(),
            feature_incompat: sb.feature_incompat.bits(),
            feature_ro_compat: sb.feature_ro_compat.bits(),
            uuid: sb.uuid,
            volume_name: sb.volume_name,
            last_mounted_dir: sb.last_mounted_dir,
            algorithm_usage_bitmap: sb.algorithm_usage_bitmap,
            prealloc_file_blocks: sb.prealloc_file_blocks,
            prealloc_dir_blocks: sb.prealloc_dir_blocks,
            padding1: sb.padding1,
            journal_uuid: sb.journal_uuid,
            journal_ino: sb.journal_ino,
            journal_dev: sb.journal_dev,
            last_orphan: sb.last_orphan,
            hash_seed: sb.hash_seed,
            def_hash_version: sb.def_hash_version,
            reserved_char_pad: sb.reserved_char_pad,
            reserved_word_pad: sb.reserved_word_pad,
            default_mount_opts: sb.default_mount_opts,
            first_meta_bg: sb.first_meta_bg,
            reserved: sb.reserved,
        }
    }
}

/// Reserved padding to fill the on-disk superblock to 1024 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct Reserved([u32; 190]);

impl Default for Reserved {
    fn default() -> Self {
        Self([0u32; 190])
    }
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::*;

    use super::*;
    use crate::fs::fs_impls::ext2::test_utils::make_valid_raw_super_block;

    #[ktest]
    fn max_file_size_matches_ext2_4k_limit() {
        let raw = make_valid_raw_super_block(1);
        let sb = SuperBlock::try_from(raw).unwrap();

        assert_eq!(sb.max_file_size(), 2_196_873_666_560);
    }

    #[ktest]
    fn sparse_super_backup_groups() {
        let mut raw = make_valid_raw_super_block(30);
        raw.feature_ro_compat = FeatureRoCompatSet::SPARSE_SUPER.bits();

        let sb = SuperBlock::try_from(raw).unwrap();
        // Group 0 is primary, not "backup".
        assert!(!sb.is_backup_group(0));
        // Group 1 is always a backup.
        assert!(sb.is_backup_group(1));
        // Powers of 3, 5, 7 are backups.
        assert!(sb.is_backup_group(3));
        assert!(sb.is_backup_group(5));
        assert!(sb.is_backup_group(7));
        assert!(sb.is_backup_group(9)); // 3^2
        assert!(sb.is_backup_group(25)); // 5^2
        // 2, 4, 6 are not backups with sparse_super.
        assert!(!sb.is_backup_group(2));
        assert!(!sb.is_backup_group(4));
        assert!(!sb.is_backup_group(6));
    }
}
