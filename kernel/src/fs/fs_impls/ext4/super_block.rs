// SPDX-License-Identifier: MPL-2.0

//! On-disk ext4 superblock parsing and the validated in-memory representation.
//!
//! Only the fields needed by the currently supported feature set are decoded.
//! Volumes requiring unsupported incompatible features are rejected.

use super::{
    block_group::RawBlockGroup,
    feature::{
        FeatureCompatSet, FeatureIncompatSet, FeatureRoCompatSet, INCOMPAT_SUPP, RO_COMPAT_SUPP,
    },
    prelude::*,
    utils::IsPowerOf,
};

/// Magic signature (`s_magic`).
pub(super) const MAGIC_NUM: u16 = 0xef53;

/// The main superblock is located at byte 1024 from the start of the device.
pub(super) const SUPER_BLOCK_OFFSET: usize = 1024;

const SUPER_BLOCK_SIZE: usize = 1024;

/// Validated, Rust-typed in-memory representation of the ext4 superblock.
///
/// Block counts use the implementation's 64-bit block-number type, although
/// the `64BIT` on-disk feature is not supported yet.
#[derive(Clone, Copy, Debug)]
pub(super) struct SuperBlock {
    inodes_count: u32,
    blocks_count: u64,
    reserved_blocks_count: u64,
    free_blocks_count: u64,
    free_inodes_count: u32,
    default_reserved_uid: u32,
    default_reserved_gid: u32,
    first_data_block: Ext4Bid,
    block_size: usize,
    nr_blocks_per_group: u32,
    nr_block_groups: u32,
    nr_inodes_per_group: u32,
    // Some decoded fields are retained for mount validation and future format
    // support even when the current data path does not use them directly.
    nr_inode_table_blocks_per_group: u32,
    inode_size: usize,
    first_ino: u32,
    rev_level: RevLevel,
    state: FsState,
    feature_compat: FeatureCompatSet,
    feature_incompat: FeatureIncompatSet,
    feature_ro_compat: FeatureRoCompatSet,
}

impl TryFrom<RawSuperBlock> for SuperBlock {
    type Error = Error;

    fn try_from(sb: RawSuperBlock) -> Result<Self> {
        if sb.magic != MAGIC_NUM {
            return_errno_with_message!(Errno::EINVAL, "bad ext4 magic number");
        }

        // The page-cache mapping assumes one filesystem block per page.
        if sb.log_block_size != 2 {
            return_errno_with_message!(Errno::EINVAL, "unsupported block size (4 KiB only)");
        }
        if sb.log_frag_size != sb.log_block_size {
            return_errno_with_message!(Errno::EINVAL, "invalid fragment size");
        }
        let block_size = BLOCK_SIZE;

        let state = FsState::from_bits_truncate(sb.state);

        let errors_behavior = ErrorsBehavior::try_from(sb.errors)
            .map_err(|_| Error::with_message(Errno::EINVAL, "invalid errors behavior"))?;
        if errors_behavior != ErrorsBehavior::Continue {
            return_errno_with_message!(Errno::EINVAL, "unsupported errors behavior");
        }

        let creator_os = OsId::try_from(sb.creator_os)
            .map_err(|_| Error::with_message(Errno::EINVAL, "invalid creator os"))?;
        if creator_os != OsId::Linux {
            return_errno_with_message!(Errno::EINVAL, "unsupported creator os");
        }

        let rev_level = RevLevel::try_from(sb.rev_level)
            .map_err(|_| Error::with_message(Errno::EINVAL, "invalid revision level"))?;
        let (first_ino, inode_size) = match rev_level {
            RevLevel::GoodOld => (11, 128usize),
            RevLevel::Dynamic => {
                let inode_size = usize::from(sb.inode_size);
                if inode_size < 128 || inode_size > block_size || !inode_size.is_power_of_two() {
                    return_errno_with_message!(Errno::EINVAL, "invalid inode size");
                }
                (sb.first_ino, inode_size)
            }
        };

        // Reject any incompatible feature that cannot be interpreted safely.
        let unsupported_incompat = sb.feature_incompat & !INCOMPAT_SUPP.bits();
        if unsupported_incompat != 0 {
            return_errno_with_message!(Errno::EINVAL, "unsupported incompatible feature");
        }
        let feature_incompat = FeatureIncompatSet::from_bits_truncate(sb.feature_incompat);
        // The directory code reads and writes each entry's `file_type` byte
        // unconditionally, so volumes without FILETYPE (including rev 0
        // volumes, whose feature words are all zero) are rejected instead of
        // misparsing the high byte of `name_len`. EXTENTS is deliberately not
        // required: ext2-format volumes map every inode through the indirect
        // engine, selected per inode by its `EXTENTS` flag.
        if !feature_incompat.contains(FeatureIncompatSet::FILETYPE) {
            return_errno_with_message!(Errno::EINVAL, "image without the filetype feature");
        }
        let feature_compat = FeatureCompatSet::from_bits_truncate(sb.feature_compat);
        // Reject unknown read-only compatible features instead of silently
        // ignoring them: there is no read-only mount mode to downgrade to, and
        // writing such a volume (e.g. one with `metadata_csum`) would corrupt
        // the very structures the feature protects.
        let unsupported_ro_compat = sb.feature_ro_compat & !RO_COMPAT_SUPP.bits();
        if unsupported_ro_compat != 0 {
            return_errno_with_message!(Errno::EINVAL, "unsupported read-only compatible feature");
        }
        let feature_ro_compat = FeatureRoCompatSet::from_bits_truncate(sb.feature_ro_compat);

        let nr_inodes_per_group = sb.inodes_per_group;
        let nr_blocks_per_group = sb.blocks_per_group;
        if nr_inodes_per_group == 0 || nr_blocks_per_group == 0 {
            return_errno_with_message!(Errno::EINVAL, "invalid group sizes");
        }

        let inodes_per_block =
            u32::try_from(block_size / inode_size).expect("filesystem block size fits u32");
        let max_bits_per_group =
            u32::try_from(block_size).expect("filesystem block size fits u32") * 8;
        if nr_inodes_per_group < inodes_per_block || nr_inodes_per_group > max_bits_per_group {
            return_errno_with_message!(Errno::EINVAL, "invalid inodes per group");
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
        let nr_block_groups =
            (blocks_count - first_data_block - 1) / nr_blocks_per_group as u64 + 1;
        let nr_block_groups = u32::try_from(nr_block_groups)
            .map_err(|_| Error::with_message(Errno::EINVAL, "too many block groups"))?;

        let max_inodes = u64::from(nr_block_groups) * u64::from(nr_inodes_per_group);
        let min_inodes = u64::from(nr_block_groups - 1) * u64::from(nr_inodes_per_group);
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

        Ok(Self {
            inodes_count: sb.inodes_count,
            blocks_count,
            reserved_blocks_count: sb.reserved_blocks_count as u64,
            free_blocks_count: sb.free_blocks_count as u64,
            free_inodes_count: sb.free_inodes_count,
            default_reserved_uid: sb.default_reserved_uid as u32,
            default_reserved_gid: sb.default_reserved_gid as u32,
            first_data_block,
            block_size,
            nr_blocks_per_group,
            nr_block_groups,
            nr_inodes_per_group,
            nr_inode_table_blocks_per_group,
            inode_size,
            first_ino,
            rev_level,
            state,
            feature_compat,
            feature_incompat,
            feature_ro_compat,
        })
    }
}

impl SuperBlock {
    pub(super) const fn block_size(&self) -> usize {
        self.block_size
    }

    pub(super) const fn inode_size(&self) -> usize {
        self.inode_size
    }

    /// Returns the fragment size. Mount validation requires it to equal the
    /// block size, so no separate field is kept.
    pub(super) const fn fragment_size(&self) -> usize {
        self.block_size
    }

    pub(super) const fn total_inodes(&self) -> u32 {
        self.inodes_count
    }

    pub(super) const fn total_blocks(&self) -> u64 {
        self.blocks_count
    }

    pub(super) const fn nr_blocks_per_group(&self) -> u32 {
        self.nr_blocks_per_group
    }

    /// Returns the first block number of a block group.
    pub(super) const fn group_first_block_no(&self, group_idx: usize) -> Ext4Bid {
        (group_idx as Ext4Bid) * (self.nr_blocks_per_group as Ext4Bid) + self.first_data_block
    }

    pub(super) const fn first_data_block(&self) -> Ext4Bid {
        self.first_data_block
    }

    pub(super) const fn nr_inodes_per_group(&self) -> u32 {
        self.nr_inodes_per_group
    }

    pub(super) const fn nr_inode_table_blocks_per_group(&self) -> u32 {
        self.nr_inode_table_blocks_per_group
    }

    pub(super) const fn first_ino(&self) -> u32 {
        self.first_ino
    }

    /// Returns the number of block groups, rounding up the last partial group.
    pub(super) fn nr_block_groups(&self) -> u32 {
        self.nr_block_groups
    }

    /// Returns the number of blocks the group-descriptor table occupies.
    pub(super) const fn group_descriptor_blocks_count(&self) -> u32 {
        let group_desc_bytes = (self.nr_block_groups as usize) * size_of::<RawBlockGroup>();
        group_desc_bytes.div_ceil(self.block_size) as u32
    }

    pub(super) const fn free_blocks_count(&self) -> u64 {
        self.free_blocks_count
    }

    /// Returns the number of blocks reserved for privileged users.
    pub(super) const fn reserved_blocks_count(&self) -> u64 {
        self.reserved_blocks_count
    }

    /// Returns the UID that may use the reserved blocks.
    pub(super) const fn default_reserved_uid(&self) -> u32 {
        self.default_reserved_uid
    }

    /// Returns the GID that may use the reserved blocks.
    pub(super) const fn default_reserved_gid(&self) -> u32 {
        self.default_reserved_gid
    }

    /// Increases the free-block counter by `n`, erroring on overflow past the
    /// total block count.
    pub(super) fn inc_free_blocks(&mut self, n: u64) -> Result<()> {
        let new_count = self
            .free_blocks_count
            .checked_add(n)
            .ok_or_else(|| Error::with_message(Errno::EIO, "free block counter overflow"))?;
        if new_count > self.blocks_count {
            return_errno_with_message!(Errno::EIO, "free block counter exceeds total blocks");
        }
        self.free_blocks_count = new_count;
        Ok(())
    }

    /// Decreases the free-block counter by `n`, erroring on underflow.
    ///
    /// Takes `&mut self` so that a write guard over `RwMutex<Dirty<SuperBlock>>`
    /// marks the superblock dirty for writeback.
    pub(super) fn dec_free_blocks(&mut self, n: u64) -> Result<()> {
        self.free_blocks_count = self
            .free_blocks_count
            .checked_sub(n)
            .ok_or_else(|| Error::with_message(Errno::EIO, "free block counter underflow"))?;
        Ok(())
    }

    pub(super) const fn free_inodes_count(&self) -> u32 {
        self.free_inodes_count
    }

    /// Increases the free-inode counter by one, erroring on overflow past the
    /// total inode count.
    pub(super) fn inc_free_inodes(&mut self) -> Result<()> {
        let new_count = self
            .free_inodes_count
            .checked_add(1)
            .ok_or_else(|| Error::with_message(Errno::EIO, "free inode counter overflow"))?;
        if new_count > self.inodes_count {
            return_errno_with_message!(Errno::EIO, "free inode counter exceeds total inodes");
        }
        self.free_inodes_count = new_count;
        Ok(())
    }

    /// Decreases the free-inode counter by one, erroring on underflow.
    ///
    /// Takes `&mut self` so that a write guard over `RwMutex<Dirty<SuperBlock>>`
    /// marks the superblock dirty for writeback.
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
    pub(super) fn total_metadata_blocks(&self) -> u64 {
        let group_desc_blocks_count = u64::from(self.group_descriptor_blocks_count());
        let mut overhead = self.first_data_block;

        for group_idx in 0..self.nr_block_groups as usize {
            if self.has_super_block(group_idx) {
                overhead = overhead.saturating_add(1 + group_desc_blocks_count);
            }
        }

        overhead.saturating_add(
            u64::from(self.nr_block_groups).saturating_mul(
                2u64.saturating_add(u64::from(self.nr_inode_table_blocks_per_group)),
            ),
        )
    }

    #[expect(dead_code)]
    pub(super) const fn state(&self) -> FsState {
        self.state
    }

    #[expect(dead_code)]
    pub(super) const fn rev_level(&self) -> RevLevel {
        self.rev_level
    }

    pub(super) const fn feature_compat(&self) -> FeatureCompatSet {
        self.feature_compat
    }

    pub(super) const fn feature_incompat(&self) -> FeatureIncompatSet {
        self.feature_incompat
    }

    #[expect(dead_code)]
    pub(super) const fn feature_ro_compat(&self) -> FeatureRoCompatSet {
        self.feature_ro_compat
    }
}

bitflags! {
    /// Filesystem state (`s_state`).
    pub(super) struct FsState: u16 {
        /// Unmounted cleanly.
        const VALID = 1 << 0;
        /// Errors detected.
        const ERROR = 1 << 1;
        /// Orphan inodes are being recovered.
        const ORPHAN = 1 << 2;
    }
}

/// Action taken when an error is detected (`s_errors`).
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

/// The ext4 revision level (`s_rev_level`).
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub(super) enum RevLevel {
    /// Original format with a fixed 128-byte inode.
    GoodOld = 0,
    /// V2 format with a dynamic inode size (ext4 uses this).
    Dynamic = 1,
}

const_assert!(size_of::<RawSuperBlock>() == SUPER_BLOCK_SIZE);

/// The on-disk superblock structure (exactly 1024 bytes).
///
/// Convert to `SuperBlock` via `TryFrom` for the validated representation.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct RawSuperBlock {
    pub inodes_count: u32,
    pub blocks_count: u32,
    pub reserved_blocks_count: u32,
    pub free_blocks_count: u32,
    pub free_inodes_count: u32,
    pub first_data_block: u32,
    /// The number to left-shift 1024 by to obtain the block size.
    pub log_block_size: u32,
    /// The number to left-shift 1024 by to obtain the fragment size.
    pub log_frag_size: u32,
    pub blocks_per_group: u32,
    pub frags_per_group: u32,
    pub inodes_per_group: u32,
    pub mtime: UnixTime,
    pub wtime: UnixTime,
    pub mnt_count: u16,
    pub max_mnt_count: u16,
    pub magic: u16,
    pub state: u16,
    pub errors: u16,
    pub min_rev_level: u16,
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
    pub(super) padding1: u16,
    pub journal_uuid: [u8; 16],
    pub journal_ino: u32,
    pub journal_dev: u32,
    pub last_orphan: u32,
    pub hash_seed: [u32; 4],
    pub def_hash_version: u8,
    pub(super) reserved_char_pad: u8,
    pub(super) reserved_word_pad: u16,
    pub default_mount_opts: u32,
    pub first_meta_bg: u32,
    pub(super) reserved: Reserved,
}

/// Unmodeled fields and reserved padding that complete the 1024-byte on-disk
/// superblock. Unsupported features are rejected before these bytes are used.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct Reserved([u32; 190]);

impl Default for Reserved {
    fn default() -> Self {
        Self([0u32; 190])
    }
}
