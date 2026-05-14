// SPDX-License-Identifier: MPL-2.0

use ostd::const_assert;

use super::prelude::*;
use crate::fs::utils::{IsPowerOf, Str16, Str64};
use crate::prelude::*;
use crate::time::UnixTime;

/// Ext4 magic number.
pub const MAGIC_NUM: u16 = 0xef53;

/// The main superblock is located at byte 1024 from the beginning of the device.
pub const SUPER_BLOCK_OFFSET: usize = 1024;

const SUPER_BLOCK_SIZE: usize = 1024;

/// The in-memory parsed Ext4 superblock.
#[derive(Clone, Copy, Debug)]
pub struct SuperBlock {
    inodes_count: u32,
    blocks_count: u64,
    free_blocks_count: u64,
    free_inodes_count: u32,
    first_data_block: u32,
    block_size: usize,
    cluster_size: usize,
    blocks_per_group: u32,
    clusters_per_group: u32,
    inodes_per_group: u32,
    magic: u16,
    state: FsState,
    rev_level: RevLevel,
    inode_size: usize,
    block_group_idx: usize,
    feature_compat: FeatureCompatSet,
    feature_incompat: FeatureInCompatSet,
    feature_ro_compat: FeatureRoCompatSet,
    uuid: [u8; 16],
    volume_name: Str16,
    last_mounted_dir: Str64,
    first_ino: u32,
    min_extra_isize: u16,
    want_extra_isize: u16,
    flags: FsFlags,
    desc_size: usize,
    s_mnt_count: u16,
    s_max_mnt_count: u16,
    s_mtime: UnixTime,
    s_wtime: UnixTime,
    s_last_check_time: UnixTime,
    s_check_interval: u32,
    s_creator_os: u32,
    s_def_resuid: u32,
    s_def_resgid: u32,
    first_meta_bg: u32,
    s_reserved_gdt_blocks: u16,
}

impl TryFrom<RawSuperBlock> for SuperBlock {
    type Error = crate::error::Error;

    fn try_from(sb: RawSuperBlock) -> Result<Self> {
        if sb.magic != MAGIC_NUM {
            return_errno_with_message!(Errno::EINVAL, "bad ext4 magic number");
        }

        let block_size = (SUPER_BLOCK_SIZE as u32) << sb.log_block_size;
        if block_size != 1024 && block_size != 2048 && block_size != 4096 {
            return_errno_with_message!(Errno::EINVAL, "unsupported ext4 block size");
        }

        let state = FsState::from_bits(sb.state)
            .ok_or(Error::with_message(Errno::EINVAL, "invalid fs state"))?;

        let rev_level = RevLevel::try_from(sb.rev_level)
            .map_err(|_| Error::with_message(Errno::EINVAL, "invalid revision level"))?;

        let feature_incompat = FeatureInCompatSet::from_bits(sb.feature_incompat).ok_or(
            Error::with_message(Errno::EINVAL, "invalid feature incompat set"),
        )?;

        // These are mandatory for ext4.
        if !feature_incompat.contains(FeatureInCompatSet::EXTENTS) {
            return_errno_with_message!(
                Errno::EINVAL,
                "ext4 requires extents feature (consider using ext2 driver)"
            );
        }

        let feature_ro_compat = FeatureRoCompatSet::from_bits(sb.feature_ro_compat).ok_or(
            Error::with_message(Errno::EINVAL, "invalid feature ro compat set"),
        )?;

        let blocks_count = if feature_incompat.contains(FeatureInCompatSet::_64BIT) {
            ((sb.blocks_count_hi as u64) << 32) | sb.blocks_count_lo as u64
        } else {
            sb.blocks_count_lo as u64
        };

        let free_blocks_count = if feature_incompat.contains(FeatureInCompatSet::_64BIT) {
            ((sb.free_blocks_count_hi as u64) << 32) | sb.free_blocks_count_lo as u64
        } else {
            sb.free_blocks_count_lo as u64
        };

        let inode_size = sb.inode_size as usize;
        if inode_size < 128 {
            return_errno_with_message!(Errno::EINVAL, "inode size too small");
        }

        let feature_compat = FeatureCompatSet::from_bits(sb.feature_compat).ok_or(
            Error::with_message(Errno::EINVAL, "invalid feature compat set"),
        )?;

        let desc_size = if sb.desc_size != 0 {
            sb.desc_size as usize
        } else {
            32 // Standard ext2/ext4 32-bit group descriptor size
        };

        Ok(Self {
            inodes_count: sb.inodes_count,
            blocks_count,
            free_blocks_count,
            free_inodes_count: sb.free_inodes_count,
            first_data_block: sb.first_data_block,
            block_size: block_size as usize,
            cluster_size: ((SUPER_BLOCK_SIZE as u64) << sb.log_cluster_size) as usize,
            blocks_per_group: sb.blocks_per_group,
            clusters_per_group: sb.clusters_per_group,
            inodes_per_group: sb.inodes_per_group,
            magic: MAGIC_NUM,
            state,
            rev_level,
            inode_size,
            block_group_idx: sb.block_group_idx as usize,
            feature_compat,
            feature_incompat,
            feature_ro_compat,
            uuid: sb.uuid,
            volume_name: sb.volume_name,
            last_mounted_dir: sb.last_mounted_dir,
            first_ino: sb.first_ino,
            min_extra_isize: sb.min_extra_isize,
            want_extra_isize: sb.want_extra_isize,
            flags: FsFlags::from_bits_retain(sb.flags),
            desc_size,
            s_mnt_count: sb.mnt_count,
            s_max_mnt_count: sb.max_mnt_count,
            s_mtime: sb.mtime,
            s_wtime: sb.wtime,
            s_last_check_time: sb.last_check_time,
            s_check_interval: sb.check_interval,
            s_creator_os: sb.creator_os,
            s_def_resuid: sb.def_resuid as u32,
            s_def_resgid: sb.def_resgid as u32,
            first_meta_bg: sb.first_meta_bg,
            s_reserved_gdt_blocks: sb.reserved_gdt_blocks,
        })
    }
}

impl SuperBlock {
    /// Returns the block size.
    pub fn block_size(&self) -> usize {
        self.block_size
    }

    /// Returns the size of inode structure.
    pub fn inode_size(&self) -> usize {
        self.inode_size
    }

    /// Returns total number of inodes.
    pub fn inodes_count(&self) -> u32 {
        self.inodes_count
    }

    /// Returns total number of blocks.
    pub fn blocks_count(&self) -> u64 {
        self.blocks_count
    }

    /// Returns the number of blocks in each block group.
    pub fn blocks_per_group(&self) -> u32 {
        self.blocks_per_group
    }

    /// Returns the number of inodes in each block group.
    pub fn inodes_per_group(&self) -> u32 {
        self.inodes_per_group
    }

    /// Returns the number of block groups.
    pub fn block_groups_count(&self) -> u32 {
        self.blocks_count.div_ceil(self.blocks_per_group as u64) as u32
    }

    /// Returns the filesystem state.
    pub fn state(&self) -> FsState {
        self.state
    }

    /// Returns the incompatible feature set.
    pub fn feature_incompat(&self) -> FeatureInCompatSet {
        self.feature_incompat
    }

    /// Returns the readonly-compatible feature set.
    pub fn feature_ro_compat(&self) -> FeatureRoCompatSet {
        self.feature_ro_compat
    }

    /// Returns the compatible feature set.
    pub fn feature_compat(&self) -> FeatureCompatSet {
        self.feature_compat
    }

    /// Returns the descriptor size.
    pub fn desc_size(&self) -> usize {
        self.desc_size
    }

    /// Checks if the block group backs up superblock and group descriptor.
    pub fn is_backup_group(&self, block_group_idx: usize) -> bool {
        if block_group_idx == 0 {
            return false;
        }
        if self
            .feature_ro_compat
            .contains(FeatureRoCompatSet::SPARSE_SUPER)
        {
            block_group_idx == 1
                || is_power_of(block_group_idx, 3)
                || is_power_of(block_group_idx, 5)
                || is_power_of(block_group_idx, 7)
        } else {
            true
        }
    }

    /// Returns the starting block id of the superblock inside a block group.
    pub fn bid(&self, block_group_idx: usize) -> u64 {
        if block_group_idx == 0 {
            return (SUPER_BLOCK_OFFSET / self.block_size) as u64;
        }
        let super_block_bid = block_group_idx as u64 * self.blocks_per_group as u64;
        super_block_bid
    }

    /// Returns the starting block id of the group descriptor table inside a block group.
    pub fn group_descriptors_bid(&self, block_group_idx: usize) -> u64 {
        let super_block_bid = self.bid(block_group_idx);
        super_block_bid + (SUPER_BLOCK_SIZE.div_ceil(self.block_size) as u64)
    }

    /// Returns the number of free blocks.
    pub fn free_blocks_count(&self) -> u64 {
        self.free_blocks_count
    }

    /// Returns the number of free inodes.
    pub fn free_inodes_count(&self) -> u32 {
        self.free_inodes_count
    }
}

bitflags! {
    /// Compatible feature set.
    pub struct FeatureCompatSet: u32 {
        const DIR_PREALLOC = 0x1;
        const IMAGIC_INODES = 0x2;
        const HAS_JOURNAL = 0x4;
        const EXT_ATTR = 0x8;
        const RESIZE_INODE = 0x10;
        const DIR_INDEX = 0x20;
        const LAZY_BG = 0x40;
        const EXCLUDE_INODE = 0x80;
        const EXCLUDE_BITMAP = 0x100;
        const SPARSE_SUPER2 = 0x200;
        const FAST_COMMIT = 0x400;
        const STABLE_INODES = 0x800;
        const ORPHAN_FILE = 0x1000;
    }
}

bitflags! {
    /// Incompatible feature set.
    pub struct FeatureInCompatSet: u32 {
        const COMPRESSION = 0x1;
        const FILETYPE = 0x2;
        const RECOVER = 0x4;
        const JOURNAL_DEV = 0x8;
        const META_BG = 0x10;
        const EXTENTS = 0x40;
        const _64BIT = 0x80;
        const MMP = 0x100;
        const FLEX_BG = 0x200;
        const EA_INODE = 0x400;
        const DIRDATA = 0x1000;
        const CSUM_SEED = 0x2000;
        const LARGEDIR = 0x4000;
        const INLINE_DATA = 0x8000;
        const ENCRYPT = 0x10000;
        const CASEFOLD = 0x20000;
    }
}

bitflags! {
    /// Readonly-compatible feature set.
    pub struct FeatureRoCompatSet: u32 {
        const SPARSE_SUPER = 0x1;
        const LARGE_FILE = 0x2;
        const BTREE_DIR = 0x4;
        const HUGE_FILE = 0x8;
        const GDT_CSUM = 0x10;
        const DIR_NLINK = 0x20;
        const EXTRA_ISIZE = 0x40;
        const HAS_SNAPSHOT = 0x80;
        const QUOTA = 0x100;
        const BIGALLOC = 0x200;
        const METADATA_CSUM = 0x400;
        const REPLICA = 0x800;
        const READONLY = 0x1000;
        const PROJECT = 0x2000;
        const VERITY = 0x8000;
        const ORPHAN_PRESENT = 0x10000;
    }
}

bitflags! {
    /// Filesystem state.
    pub struct FsState: u16 {
        const VALID = 0x1;
        const ERROR = 0x2;
        const ORPHAN_RECOVERY = 0x4;
    }
}

bitflags! {
    /// Miscellaneous filesystem flags.
    pub struct FsFlags: u32 {
        const SIGNED_DIR_HASH = 0x1;
        const UNSIGNED_DIR_HASH = 0x2;
        const TEST_DEV = 0x4;
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub enum RevLevel {
    GoodOld = 0,
    Dynamic = 1,
}

const_assert!(size_of::<RawSuperBlock>() == SUPER_BLOCK_SIZE);

/// The on-disk Ext4 superblock structure.
///
/// It must be exactly 1024 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct RawSuperBlock {
    pub inodes_count: u32,
    pub blocks_count_lo: u32,
    pub r_blocks_count_lo: u32,
    pub free_blocks_count_lo: u32,
    pub free_inodes_count: u32,
    pub first_data_block: u32,
    /// log2(block_size) - 10
    pub log_block_size: u32,
    /// log2(cluster_size) - 10
    pub log_cluster_size: u32,
    pub blocks_per_group: u32,
    pub clusters_per_group: u32,
    pub inodes_per_group: u32,
    pub mtime: UnixTime,
    pub wtime: UnixTime,
    pub mnt_count: u16,
    pub max_mnt_count: u16,
    pub magic: u16,
    pub state: u16,
    pub errors: u16,
    pub minor_rev_level: u16,
    pub last_check_time: UnixTime,
    pub check_interval: u32,
    pub creator_os: u32,
    pub rev_level: u32,
    pub def_resuid: u16,
    pub def_resgid: u16,
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
    pub prealloc_blocks: u8,
    pub prealloc_dir_blocks: u8,
    pub reserved_gdt_blocks: u16,
    pub journal_uuid: [u8; 16],
    pub journal_inum: u32,
    pub journal_dev: u32,
    pub last_orphan: u32,
    pub hash_seed: [u32; 4],
    pub def_hash_version: u8,
    pub jnl_backup_type: u8,
    pub desc_size: u16,
    pub default_mount_opts: u32,
    pub first_meta_bg: u32,
    pub mkfs_time: u32,
    pub jnl_blocks: [u32; 17],
    // --- 64-bit support fields (offset 0x150) ---
    pub blocks_count_hi: u32,
    pub r_blocks_count_hi: u32,
    pub free_blocks_count_hi: u32,
    pub min_extra_isize: u16,
    pub want_extra_isize: u16,
    pub flags: u32,
    pub raid_stride: u16,
    pub mmp_interval: u16,
    pub mmp_block: u64,
    pub raid_stripe_width: u32,
    pub log_groups_per_flex: u8,
    pub checksum_type: u8,
    pub reserved_pad: u16,
    pub kbytes_written: u64,
    pub snapshot_inum: u32,
    pub snapshot_id: u32,
    pub snapshot_r_blocks_count: u64,
    pub snapshot_list: u32,
    pub error_count: u32,
    pub first_error_time: u32,
    pub first_error_ino: u32,
    pub first_error_block: u64,
    pub first_error_func: [u8; 32],
    pub first_error_line: u32,
    pub last_error_time: u32,
    pub last_error_ino: u32,
    pub last_error_line: u32,
    pub last_error_block: u64,
    pub last_error_func: [u8; 32],
    pub mount_opts: [u8; 64],
    pub usr_quota_inum: u32,
    pub grp_quota_inum: u32,
    pub overhead_blocks: u32,
    pub backup_bgs: [u32; 2],
    pub encrypt_algos: [u8; 4],
    pub encrypt_pw_salt: [u8; 16],
    pub lpf_ino: u32,
    pub prj_quota_inum: u32,
    pub checksum_seed: u32,
    pub wtime_hi: u8,
    pub mtime_hi: u8,
    pub mkfs_time_hi: u8,
    pub lastcheck_hi: u8,
    pub first_error_time_hi: u8,
    pub last_error_time_hi: u8,
    pub first_error_errcode: u8,
    pub last_error_errcode: u8,
    pub encoding: u16,
    pub encoding_flags: u16,
    pub orphan_file_inum: u32,
    reserved: Reserved,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct Reserved([u32; 94]);

impl Default for Reserved {
    fn default() -> Self {
        Self([0u32; 94])
    }
}
