pub const EXFAT_ROOT_INO: usize = 1;
pub const EXFAT_CLUSTERS_UNTRACKED: u32 = !0;

// NLS lossy flags
pub const NLS_NAME_NO_LOSSY: u8 = 0;
pub const NLS_NAME_LOSSY: u8 = 1;
pub const NLS_NAME_OVERLEN: u8 = 2; 

pub const EXFAT_HASH_BITS: u8 = 8;
pub const EXFAT_HASH_SIZE: u32 = 1 << EXFAT_HASH_BITS;

// Entry set indexes
pub const ES_2_ENTRIES: u32 = 2;
pub const ES_ALL_ENTRIES: u32 = 0;

pub const ES_IDX_FILE: usize = 0;
pub const ES_IDX_STREAM: usize = 1;
pub const ES_IDX_FIRST_FILENAME: usize = 2;


// Other pub constants 
pub const DIR_DELETED: u32 = 0xFFFFFFF7;
pub const TYPE_UNUSED		:u16 = 0x0000;
pub const TYPE_DELETED		:u16 = 0x0001;
pub const TYPE_INVALID		:u16 = 0x0002;
pub const TYPE_CRITICAL_PRI	:u16 = 0x0100;
pub const TYPE_BITMAP		:u16 = 0x0101;
pub const TYPE_UPCASE		:u16 = 0x0102;
pub const TYPE_VOLUME		:u16 = 0x0103;
pub const TYPE_DIR		    :u16 = 0x0104;
pub const TYPE_FILE		    :u16 = 0x011F;
pub const TYPE_CRITICAL_SEC	:u16 = 0x0200;
pub const TYPE_STREAM		:u16 = 0x0201;
pub const TYPE_EXTEND		:u16 = 0x0202;
pub const TYPE_ACL		    :u16 = 0x0203;
pub const TYPE_BENIGN_PRI	:u16 = 	0x0400;
pub const TYPE_GUID		    :u16 = 0x0401;
pub const TYPE_PADDING		:u16 = 0x0402;
pub const TYPE_ACLTAB		:u16 = 0x0403;
pub const TYPE_BENIGN_SEC	:u16 = 	0x0800;
pub const TYPE_VENDOR_EXT	:u16 = 	0x0801;
pub const TYPE_VENDOR_ALLOC	:u16 = 0x0802;
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

pub const EXFAT_EOF_CLUSTER: u32 = 0xFFFFFFFF;
pub const EXFAT_BAD_CLUSTER: u32 = 0xFFFFFFF7;
pub const EXFAT_FREE_CLUSTER: u32 = 0;
// Cluster 0, 1 are reserved, the first cluster is 2 in the cluster heap.
pub const EXFAT_RESERVED_CLUSTERS: u32 = 2;
pub const EXFAT_FIRST_CLUSTER: u32 = 2;

// AllocationPossible and NoFatChain field in GeneralSecondaryFlags Field
pub const ALLOC_POSSIBLE: u8 = 0x01;
pub const ALLOC_FAT_CHAIN: u8 = 0x01;
pub const ALLOC_NO_FAT_CHAIN: u8 = 0x03;

pub const DENTRY_SIZE: usize = 32; // directory entry size
pub const DENTRY_SIZE_BITS: u32 = 5;
// exFAT allows 8388608(256MB) directory entries
pub const MAX_EXFAT_DENTRIES: u32 = 8388608;

// dentry types
pub const EXFAT_UNUSED: u8 = 0x00; // end of directory
pub const EXFAT_DELETE: u8 = !0x80;
pub const IS_EXFAT_DELETED: fn(x: u8) -> bool = |x| (x < 0x80); // deleted file (0x01~0x7F)
pub const EXFAT_INVAL: u8 = 0x80; // invalid value
pub const EXFAT_BITMAP: u8 = 0x81; // allocation bitmap
pub const EXFAT_UPCASE: u8 = 0x82; // upcase table
pub const EXFAT_VOLUME: u8 = 0x83; // volume label
pub const EXFAT_FILE: u8 = 0x85; // file or dir
pub const EXFAT_GUID: u8 = 0xA0;
pub const EXFAT_PADDING: u8 = 0xA1;
pub const EXFAT_ACLTAB: u8 = 0xA2;
pub const EXFAT_STREAM: u8 = 0xC0; // stream entry
pub const EXFAT_NAME: u8 = 0xC1; // file name entry
pub const EXFAT_ACL: u8 = 0xC2; // acl entry
pub const EXFAT_VENDOR_EXT: u8 = 0xE0; // vendor extension entry
pub const EXFAT_VENDOR_ALLOC: u8 = 0xE1; // vendor allocation entry

pub const IS_EXFAT_CRITICAL_PRI: fn(x: u8) -> bool = |x| (x < 0xA0);
pub const IS_EXFAT_BENIGN_PRI: fn(x: u8) -> bool = |x| (x < 0xC0);
pub const IS_EXFAT_CRITICAL_SEC: fn(x: u8) -> bool = |x| (x < 0xE0);

// checksum types
pub const CS_DIR_ENTRY: u8 = 0;
pub const CS_BOOT_SECTOR: u8 = 1;
pub const CS_DEFAULT: u8 = 2;

// file attributes
pub const ATTR_READONLY: u16 = 0x0001;
pub const ATTR_HIDDEN: u16 = 0x0002;
pub const ATTR_SYSTEM: u16 = 0x0004;
pub const ATTR_VOLUME: u16 = 0x0008;
pub const ATTR_SUBDIR: u16 = 0x0010;
pub const ATTR_ARCHIVE: u16 = 0x0020;

pub const ATTR_RWMASK: u16 = ATTR_HIDDEN | ATTR_SYSTEM | ATTR_VOLUME | ATTR_SUBDIR | ATTR_ARCHIVE;


pub const EXFAT_FILE_NAME_LEN: usize = 15;

pub const EXFAT_MIN_SECT_SIZE_BITS: u8 = 9;
pub const EXFAT_MAX_SECT_SIZE_BITS: u8 = 12;

// Timestamp pub constants
pub const EXFAT_MIN_TIMESTAMP_SECS: u64 = 315532800;
pub const EXFAT_MAX_TIMESTAMP_SECS: u64 = 4354819199;
pub const FAT_ENT_SIZE:u32 = 4;
pub const FAT_ENT_SIZE_BITS:u32 = 2;

pub const EXFAT_TZ_VALID: u8 = 1<<7;

pub const EXFAT_FILE_MIMIMUM_DENTRY: usize = 3;

// UpcaseTable pub constants
pub const UPCASE_MANDATORY_SIZE: usize = 128;
pub const UNICODE_SIZE: usize = 2;
