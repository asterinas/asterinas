pub const EXFAT_CLUSTERS_UNTRACKED: u32 = !0;

// NLS lossy flags
pub const NLS_NAME_NO_LOSSY: u8 = 0;
pub const NLS_NAME_LOSSY: u8 = 1;
pub const NLS_NAME_OVERLEN: u8 = 2;

pub const EXFAT_HASH_BITS: u8 = 8;
pub const EXFAT_HASH_SIZE: u32 = 1 << EXFAT_HASH_BITS;

// Other pub constants
pub const DIR_DELETED: u32 = 0xFFFFFFF7;

// Other pub constants

pub const MAX_CHARSET_SIZE: usize = 6;
pub const MAX_NAME_LENGTH: usize = 255;
pub const MAX_VFSNAME_BUF_SIZE: usize = (MAX_NAME_LENGTH + 1) * MAX_CHARSET_SIZE;

pub const EXFAT_HINT_NONE: isize = -1;
pub const EXFAT_MIN_SUBDIR: usize = 2;

pub const BOOT_SIGNATURE: u16 = 0xAA55;
pub const EXBOOT_SIGNATURE: u32 = 0xAA550000;
pub const STR_EXFAT: &str = "EXFAT   "; // size should be 8

pub const EXFAT_MAX_FILE_LEN: u8 = 255;

pub const VOLUME_DIRTY: u16 = 0x0002;
pub const MEDIA_FAILURE: u16 = 0x0004;

// Cluster 0, 1 are reserved, the first cluster is 2 in the cluster heap.

pub const EXFAT_FIRST_CLUSTER: u32 = 2;

// exFAT allows 8388608(256MB) directory entries
pub const MAX_EXFAT_DENTRIES: u32 = 8388608;

pub const EXFAT_FILE_NAME_LEN: usize = 15;

pub const EXFAT_MIN_SECT_SIZE_BITS: u8 = 9;
pub const EXFAT_MAX_SECT_SIZE_BITS: u8 = 12;

// Timestamp pub constants
pub const EXFAT_MIN_TIMESTAMP_SECS: u64 = 315532800;
pub const EXFAT_MAX_TIMESTAMP_SECS: u64 = 4354819199;

// UpcaseTable pub constants
pub const UPCASE_MANDATORY_SIZE: usize = 128;
pub const UNICODE_SIZE: usize = 2;
