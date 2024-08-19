// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]
pub(super) const ROOT_INODE_HASH: usize = 0;

// Other pub(super) constants
pub(super) const MAX_CHARSET_SIZE: usize = 6;
pub(super) const MAX_NAME_LENGTH: usize = 255;
pub(super) const MAX_VFSNAME_BUF_SIZE: usize = (MAX_NAME_LENGTH + 1) * MAX_CHARSET_SIZE;

pub(super) const BOOT_SIGNATURE: u16 = 0xAA55;
pub(super) const EXBOOT_SIGNATURE: u32 = 0xAA550000;
pub(super) const STR_EXFAT: &str = "EXFAT   "; // size should be 8

pub(super) const VOLUME_DIRTY: u16 = 0x0002;
pub(super) const MEDIA_FAILURE: u16 = 0x0004;

// Cluster 0, 1 are reserved, the first cluster is 2 in the cluster heap.
pub(super) const EXFAT_RESERVED_CLUSTERS: u32 = 2;
pub(super) const EXFAT_FIRST_CLUSTER: u32 = 2;

// exFAT allows 8388608(256MB) directory entries
pub(super) const EXFAT_MAX_DENTRIES: u32 = 8388608;

pub(super) const EXFAT_FILE_NAME_LEN: usize = 15;

pub(super) const EXFAT_MIN_SECT_SIZE_BITS: u8 = 9;
pub(super) const EXFAT_MAX_SECT_SIZE_BITS: u8 = 12;

// Timestamp constants
pub(super) const EXFAT_MIN_TIMESTAMP_SECS: u64 = 315532800;
pub(super) const EXFAT_MAX_TIMESTAMP_SECS: u64 = 4354819199;

pub(super) const UNICODE_SIZE: usize = 2;
