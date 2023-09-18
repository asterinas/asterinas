use super::prelude::*;

const EXT2_MAGIC: u16 = 0xef53;
const EXT2_DYNAMIC_REV: u32 = 1;
const OS_LINUX: u32 = 0;

/// The Superblock is located at byte 1024 from the beginning of the device.
pub const SUPER_BLOCK_OFFSET: usize = 1024;

/// The Superblock contains all information about the layout of the filesystem.
///
/// The Superblock is always located at byte 1024 from the beginning of the device
/// and is exactly 1024 bytes in length.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct Ext2SuperBlock {
    /// Total number of inodes.
    pub inodes_count: u32,
    /// Total number of blocks.
    pub blocks_count: u32,
    /// Total number of reserved blocks.
    pub reserved_blocks_count: u32,
    /// Total number of free blocks.
    pub free_blocks_count: u32,
    /// Total number of free inodes.
    pub free_inodes_count: u32,
    /// First data block.
    pub first_data_block: u32,
    /// Block size. The number to shift 1024 to the left by to obtain the size.
    pub log_block_size: u32,
    /// Fragment size. The number to shift 1024 to the left by to obtain the size.
    pub log_frag_size: u32,
    /// Number of blocks in each block group.
    pub blocks_per_group: u32,
    /// Number of fragments in each block group.
    pub frags_per_group: u32,
    /// Number of inodes in each block group.
    pub inodes_per_group: u32,
    /// Mount time.
    pub mtime: u32,
    /// Write time.
    pub wtime: u32,
    /// Mount count.
    pub mnt_count: u16,
    /// Maximal mount count.
    pub max_mnt_count: u16,
    /// Magic signature.
    pub magic: u16,
    /// File system state.
    pub state: u16,
    /// Behaviour when detecting errors.
    pub errors: u16,
    /// Minor revision level.
    pub min_rev_level: u16,
    /// Time of last check.
    pub last_check: u32,
    /// Interval between checks.
    pub check_interval: u32,
    /// Creator OS ID.
    pub creator_os: u32,
    /// Revision level.
    pub rev_level: u32,
    /// Default uid for reserved blocks.
    pub def_resuid: u16,
    /// Default gid for reserved blocks.
    pub def_resgid: u16,
    //
    // These fields are for EXT2_DYNAMIC_REV superblocks only.
    //
    /// First non-reserved inode number.
    pub first_ino: u32,
    /// Size of inode structure.
    pub inode_size: u16,
    /// Block group that this superblock is part of (if backup copy).
    pub block_group: u16,
    /// Compatible feature set.
    pub feature_compat: u32,
    /// Incompatible feature set.
    pub feature_incompat: u32,
    /// Readonly-compatible feature set.
    pub feature_ro_compat: u32,
    /// 128-bit uuid for volume.
    pub uuid: [u8; 16],
    /// Volume name.
    pub volume_name: [u8; 16],
    /// Directory where last mounted.
    pub last_mounted: [u8; 64],
    /// Compression algorithms used.
    pub algorithm_usage_bitmap: u32,
    /// Number of blocks to preallocate for files.
    pub prealloc_file_blocks: u8,
    /// Number of blocks to preallocate for directories.
    pub prealloc_dir_blocks: u8,
    padding1: u16,
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
    reserved: [u32; 190],
}

impl Ext2SuperBlock {
    /// Validates the super block.
    pub(super) fn validate(&self) -> Result<()> {
        if self.magic != EXT2_MAGIC {
            return Err(Error::BadMagic);
        }
        if self.rev_level != EXT2_DYNAMIC_REV {
            return Err(Error::BadRevision);
        }
        if self.creator_os != OS_LINUX {
            return Err(Error::BadCreaterOS);
        }

        Ok(())
    }

    /// Returns the block size.
    pub fn block_size(&self) -> usize {
        1024 << self.log_block_size
    }

    /// Returns the fragment size.
    pub fn fragment_size(&self) -> usize {
        1024 << self.log_frag_size
    }

    /// Returns the feature of ro compat.
    pub fn feature_ro_compat(&self) -> FeatureRoCompatSet {
        FeatureRoCompatSet::from_bits_truncate(self.feature_ro_compat)
    }

    /// Returns the number of block groups.
    pub fn block_groups_count(&self) -> u32 {
        self.blocks_count / self.blocks_per_group
    }

    /// Returns the starting block id of block group descripter table.
    pub(super) fn block_group_descriptors_bid(&self) -> BlockId {
        if self.block_group == 0 {
            let bid = (SUPER_BLOCK_OFFSET + core::mem::size_of::<Self>())
                .div_ceil(self.block_size()) as u32;
            BlockId::new(bid)
        } else {
            let backup_bid = self.block_group as u32 * self.blocks_per_group;
            BlockId::new(backup_bid + 1)
        }
    }

    /// Increase the number of free blocks.
    pub(super) fn inc_free_blocks(&mut self) {
        self.free_blocks_count += 1;
    }

    /// Decrease the number of free blocks.
    pub(super) fn dec_free_blocks(&mut self) {
        debug_assert!(self.free_blocks_count > 0);
        self.free_blocks_count -= 1;
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
    pub(super) fn is_backup_block_group(&self, idx: u32) -> bool {
        if idx == 0 {
            false
        } else if self
            .feature_ro_compat()
            .contains(FeatureRoCompatSet::SPARSE_SUPER)
        {
            // The backup groups chosen are 1 and powers of 3, 5 and 7.
            idx == 1 || idx.is_power_of(3) || idx.is_power_of(5) || idx.is_power_of(7)
        } else {
            true
        }
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
