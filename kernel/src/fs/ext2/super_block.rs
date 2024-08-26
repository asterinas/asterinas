// SPDX-License-Identifier: MPL-2.0

use super::{inode::RawInode, prelude::*};

/// The magic number of Ext2.
pub const MAGIC_NUM: u16 = 0xef53;

/// The main superblock is located at byte 1024 from the beginning of the device.
pub const SUPER_BLOCK_OFFSET: usize = 1024;

const SUPER_BLOCK_SIZE: usize = 1024;

/// The in-memory rust superblock.
///
/// It contains all information about the layout of the Ext2.
#[derive(Clone, Copy, Debug)]
pub struct SuperBlock {
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
    first_data_block: Bid,
    /// Block size.
    block_size: usize,
    /// Fragment size.
    frag_size: usize,
    /// Number of blocks in each block group.
    blocks_per_group: u32,
    /// Number of fragments in each block group.
    frags_per_group: u32,
    /// Number of inodes in each block group.
    inodes_per_group: u32,
    /// Mount time.
    mtime: UnixTime,
    /// Write time.
    wtime: UnixTime,
    /// Mount count.
    mnt_count: u16,
    /// Maximal mount count.
    max_mnt_count: u16,
    /// Magic signature.
    magic: u16,
    /// Filesystem state.
    state: FsState,
    /// Behaviour when detecting errors.
    errors_behaviour: ErrorsBehaviour,
    /// Time of last check.
    last_check_time: UnixTime,
    /// Interval between checks.
    check_interval: Duration,
    /// Creator OS ID.
    creator_os: OsId,
    /// Revision level.
    rev_level: RevLevel,
    /// Default uid for reserved blocks.
    def_resuid: u32,
    /// Default gid for reserved blocks.
    def_resgid: u32,
    //
    // These fields are valid for RevLevel::Dynamic only.
    //
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
    /// Readonly-compatible feature set.
    feature_ro_compat: FeatureRoCompatSet,
    /// 128-bit uuid for volume.
    uuid: [u8; 16],
    /// Volume name.
    volume_name: Str16,
    /// Directory where last mounted.
    last_mounted_dir: Str64,
    ///
    /// This fields are valid if the FeatureCompatSet::DIR_PREALLOC is set.
    ///
    /// Number of blocks to preallocate for files.
    prealloc_file_blocks: u8,
    /// Number of blocks to preallocate for directories.
    prealloc_dir_blocks: u8,
}

impl TryFrom<RawSuperBlock> for SuperBlock {
    type Error = crate::error::Error;

    fn try_from(sb: RawSuperBlock) -> Result<Self> {
        Ok(Self {
            inodes_count: sb.inodes_count,
            blocks_count: sb.blocks_count,
            reserved_blocks_count: sb.reserved_blocks_count,
            free_blocks_count: sb.free_blocks_count,
            free_inodes_count: sb.free_inodes_count,
            first_data_block: Bid::new(sb.first_data_block as _),
            block_size: 1024 << sb.log_block_size,
            frag_size: 1024 << sb.log_frag_size,
            blocks_per_group: sb.blocks_per_group,
            frags_per_group: sb.frags_per_group,
            inodes_per_group: sb.inodes_per_group,
            mtime: sb.mtime,
            wtime: sb.wtime,
            mnt_count: sb.mnt_count,
            max_mnt_count: sb.max_mnt_count,
            magic: {
                if sb.magic != MAGIC_NUM {
                    return_errno_with_message!(Errno::EINVAL, "bad ext2 magic number");
                }
                MAGIC_NUM
            },
            state: {
                let state = FsState::try_from(sb.state)
                    .map_err(|_| Error::with_message(Errno::EINVAL, "invalid fs state"))?;
                if state == FsState::Corrupted {
                    return_errno_with_message!(Errno::EUCLEAN, "fs is corrupted");
                }
                state
            },
            errors_behaviour: ErrorsBehaviour::try_from(sb.errors)
                .map_err(|_| Error::with_message(Errno::EINVAL, "invalid errors behaviour"))?,
            last_check_time: sb.last_check_time,
            check_interval: Duration::from_secs(sb.check_interval as _),
            creator_os: {
                let os_id = OsId::try_from(sb.creator_os)
                    .map_err(|_| Error::with_message(Errno::EINVAL, "invalid creator os"))?;
                if os_id != OsId::Linux {
                    return_errno_with_message!(Errno::EINVAL, "not supported os id");
                }
                OsId::Linux
            },
            rev_level: {
                let rev_level = RevLevel::try_from(sb.rev_level)
                    .map_err(|_| Error::with_message(Errno::EINVAL, "invalid revision level"))?;
                if rev_level != RevLevel::Dynamic {
                    return_errno_with_message!(Errno::EINVAL, "not supported rev level");
                }
                RevLevel::Dynamic
            },
            def_resuid: sb.def_resuid as _,
            def_resgid: sb.def_resgid as _,
            first_ino: sb.first_ino,
            inode_size: {
                let inode_size = sb.inode_size as _;
                if inode_size < core::mem::size_of::<RawInode>() {
                    return_errno_with_message!(Errno::EINVAL, "inode size is too small");
                }
                inode_size
            },
            block_group_idx: sb.block_group_idx as _,
            feature_compat: FeatureCompatSet::from_bits(sb.feature_compat).ok_or(
                Error::with_message(Errno::EINVAL, "invalid feature compat set"),
            )?,
            feature_incompat: FeatureInCompatSet::from_bits(sb.feature_incompat).ok_or(
                Error::with_message(Errno::EINVAL, "invalid feature incompat set"),
            )?,
            feature_ro_compat: FeatureRoCompatSet::from_bits(sb.feature_ro_compat).ok_or(
                Error::with_message(Errno::EINVAL, "invalid feature ro compat set"),
            )?,
            uuid: sb.uuid,
            volume_name: sb.volume_name,
            last_mounted_dir: sb.last_mounted_dir,
            prealloc_file_blocks: sb.prealloc_file_blocks,
            prealloc_dir_blocks: sb.prealloc_dir_blocks,
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

    /// Returns the fragment size.
    pub fn fragment_size(&self) -> usize {
        self.frag_size
    }

    /// Returns total number of inodes.
    pub fn total_inodes(&self) -> u32 {
        self.inodes_count
    }

    /// Returns total number of blocks.
    pub fn total_blocks(&self) -> u32 {
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
        self.blocks_count / self.blocks_per_group
    }

    /// Returns the filesystem state.
    pub fn state(&self) -> FsState {
        self.state
    }

    /// Returns the revision level.
    pub fn rev_level(&self) -> RevLevel {
        self.rev_level
    }

    /// Returns the compatible feature set.
    pub fn feature_compat(&self) -> FeatureCompatSet {
        self.feature_compat
    }

    /// Returns the incompatible feature set.
    pub fn feature_incompat(&self) -> FeatureInCompatSet {
        self.feature_incompat
    }

    /// Returns the readonly-compatible feature set.
    pub fn feature_ro_compat(&self) -> FeatureRoCompatSet {
        self.feature_ro_compat
    }

    /// Returns the number of free blocks.
    pub fn free_blocks_count(&self) -> u32 {
        self.free_blocks_count
    }

    /// Increase the number of free blocks.
    pub(super) fn inc_free_blocks(&mut self, count: u32) {
        self.free_blocks_count = self.free_blocks_count.checked_add(count).unwrap();
    }

    /// Decrease the number of free blocks.
    pub(super) fn dec_free_blocks(&mut self, count: u32) {
        self.free_blocks_count = self.free_blocks_count.checked_sub(count).unwrap();
    }

    /// Returns the number of free inodes.
    pub fn free_inodes_count(&self) -> u32 {
        self.free_inodes_count
    }

    /// Increase the number of free inodes.
    pub(super) fn inc_free_inodes(&mut self) {
        self.free_inodes_count += 1;
    }

    /// Decrease the number of free inodes.
    pub(super) fn dec_free_inodes(&mut self) {
        debug_assert!(self.free_inodes_count > 0);
        self.free_inodes_count -= 1;
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

    /// Returns the starting block id of the super block
    /// inside the block group pointed by `block_group_idx`.
    ///
    /// # Panics
    ///
    /// If `block_group_idx` is neither 0 nor a backup block group index,
    /// then the method panics.
    pub(super) fn bid(&self, block_group_idx: usize) -> Bid {
        if block_group_idx == 0 {
            let bid = (SUPER_BLOCK_OFFSET / self.block_size) as u64;
            return Bid::new(bid);
        }

        assert!(self.is_backup_group(block_group_idx));
        let super_block_bid = block_group_idx * (self.blocks_per_group as usize);
        Bid::new(super_block_bid as u64)
    }

    /// Returns the starting block id of the block group descriptor table
    /// inside the block group pointed by `block_group_idx`.
    ///
    /// # Panics
    ///
    /// If `block_group_idx` is neither 0 nor a backup block group index,
    /// then the method panics.
    pub(super) fn group_descriptors_bid(&self, block_group_idx: usize) -> Bid {
        let super_block_bid = self.bid(block_group_idx);
        super_block_bid + (SUPER_BLOCK_SIZE.div_ceil(self.block_size) as u64)
    }
}

bitflags! {
    /// Compatible feature set.
    pub struct FeatureCompatSet: u32 {
        /// Preallocate some number of blocks to a directory when creating a new one
        const DIR_PREALLOC = 1 << 0;
        /// AFS server inodes exist
        const IMAGIC_INODES = 1 << 1;
        /// File system has a journal
        const HAS_JOURNAL = 1 << 2;
        /// Inodes have extended attributes
        const EXT_ATTR = 1 << 3;
        /// File system can resize itself for larger partitions
        const RESIZE_INO = 1 << 4;
        /// Directories use hash index
        const DIR_INDEX = 1 << 5;
    }
}

bitflags! {
    /// Incompatible feature set.
    pub struct FeatureInCompatSet: u32 {
        /// Compression is used
        const COMPRESSION = 1 << 0;
        /// Directory entries contain a type field
        const FILETYPE = 1 << 1;
        /// File system needs to replay its journal
        const RECOVER = 1 << 2;
        /// File system uses a journal device
        const JOURNAL_DEV = 1 << 3;
        /// Metablock block group
        const META_BG = 1 << 4;
    }
}

bitflags! {
    /// Readonly-compatible feature set.
    pub struct FeatureRoCompatSet: u32 {
        /// Sparse superblocks and group descriptor tables
        const SPARSE_SUPER = 1 << 0;
        /// File system uses a 64-bit file size
        const LARGE_FILE = 1 << 1;
        /// Directory contents are stored in the form of a Binary Tree
        const BTREE_DIR = 1 << 2;
    }
}

#[repr(u16)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, TryFromInt)]
pub enum FsState {
    /// Unmounted cleanly
    Valid = 1,
    /// Errors detected
    Err = 2,
    /// Filesystem is corrupted (EUCLEAN)
    Corrupted = 117,
}

#[repr(u16)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, TryFromInt)]
pub enum ErrorsBehaviour {
    /// Continue execution
    Continue = 1,
    // Remount fs read-only
    RemountReadonly = 2,
    // Should panic
    Panic = 3,
}

impl Default for ErrorsBehaviour {
    fn default() -> Self {
        Self::Continue
    }
}

#[repr(u32)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, TryFromInt)]
pub enum OsId {
    Linux = 0,
    Hurd = 1,
    Masix = 2,
    FreeBSD = 3,
    Lites = 4,
}

#[repr(u32)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, TryFromInt)]
pub enum RevLevel {
    /// The good old (original) format.
    GoodOld = 0,
    /// V2 format with dynamic inode size.
    Dynamic = 1,
}

const_assert!(core::mem::size_of::<RawSuperBlock>() == SUPER_BLOCK_SIZE);

/// The raw superblock, it must be exactly 1024 bytes in length.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Default)]
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
    pub prealloc_file_blocks: u8,
    pub prealloc_dir_blocks: u8,
    padding1: u16,
    ///
    /// This fields are for journaling support in Ext3.
    ///
    /// Uuid of journal superblock.
    pub journal_uuid: [u8; 16],
    /// Inode number of journal file.
    pub journal_ino: u32,
    /// Device number of journal file.
    pub journal_dev: u32,
    /// Start of list of inodes to delete.
    pub last_orphan: u32,
    /// HTREE hash seed.
    pub hash_seed: [u32; 4],
    /// Default hash version to use
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
            first_data_block: sb.first_data_block.to_raw() as u32,
            log_block_size: (sb.block_size >> 11) as u32,
            log_frag_size: (sb.frag_size >> 11) as u32,
            blocks_per_group: sb.blocks_per_group,
            frags_per_group: sb.frags_per_group,
            inodes_per_group: sb.inodes_per_group,
            mtime: sb.mtime,
            wtime: sb.wtime,
            mnt_count: sb.mnt_count,
            max_mnt_count: sb.max_mnt_count,
            magic: sb.magic,
            state: sb.state as u16,
            errors: sb.errors_behaviour as u16,
            last_check_time: sb.last_check_time,
            check_interval: sb.check_interval.as_secs() as u32,
            creator_os: sb.creator_os as u32,
            rev_level: sb.rev_level as u32,
            def_resuid: sb.def_resuid as u16,
            def_resgid: sb.def_resgid as u16,
            first_ino: sb.first_ino,
            inode_size: sb.inode_size as u16,
            block_group_idx: sb.block_group_idx as u16,
            feature_compat: sb.feature_compat.bits(),
            feature_incompat: sb.feature_incompat.bits(),
            feature_ro_compat: sb.feature_ro_compat.bits(),
            uuid: sb.uuid,
            volume_name: sb.volume_name,
            last_mounted_dir: sb.last_mounted_dir,
            prealloc_file_blocks: sb.prealloc_file_blocks,
            prealloc_dir_blocks: sb.prealloc_dir_blocks,
            ..Default::default()
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct Reserved([u32; 190]);

impl Default for Reserved {
    fn default() -> Self {
        Self([0u32; 190])
    }
}
