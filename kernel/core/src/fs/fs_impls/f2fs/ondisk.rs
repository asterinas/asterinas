// SPDX-License-Identifier: MPL-2.0

//! On-disk structures of the F2FS (Flash-Friendly File System).
//!
//! This module contains the raw byte-level definitions that are needed to
//! interpret an F2FS volume. All multi-byte integers are little-endian and
//! structures are packed, so the code below reads values with explicit
//! offsets instead of transmuting byte buffers into `#[repr(C)]` types.
//!
//! # References
//!
//! - Linux `include/linux/f2fs_fs.h`
//! - Linux `fs/f2fs/f2fs.h`

use crate::prelude::*;

/// The F2FS magic number, found in the superblock and the checkpoint.
pub(super) const F2FS_SUPER_MAGIC: u32 = 0xf2f5_2010;

/// The superblock resides at byte offset 1024 of the device.
pub(super) const F2FS_SUPER_OFFSET: usize = 1024;

/// F2FS only supports a block size equal to the page size.
pub(super) const F2FS_BLKSIZE: usize = 4096;

/// log2 of the supported block size.
pub(super) const F2FS_LOG_BLKSIZE: u32 = 12;

/// Address pointers stored in an inode (`DEF_ADDRS_PER_INODE`).
pub(super) const DEF_ADDRS_PER_INODE: usize = 923;

/// Address pointers stored in a direct node block (`DEF_ADDRS_PER_BLOCK`).
pub(super) const ADDRS_PER_BLOCK: usize = 1018;

/// Node ids stored in an inode or an indirect node block.
pub(super) const NIDS_PER_INODE: usize = 5;

/// NAT entries stored in one NAT block.
pub(super) const NAT_ENTRY_PER_BLOCK: usize = 455;

/// Size of one on-disk NAT entry.
pub(super) const NAT_ENTRY_SIZE: usize = 9;

/// Size of one on-disk dentry entry.
pub(super) const DENTRY_SIZE: usize = 11;

/// Bytes of one filename slot.
pub(super) const SLOT_LEN: usize = 8;

/// Dentries stored in a regular dentry block.
pub(super) const NR_DENTRY_IN_BLOCK: usize = 214;

/// Bitmap size of a regular dentry block.
pub(super) const DENTRY_BITMAP_SIZE: usize = 27;

/// Reserved bytes that follow the regular dentry bitmap.
pub(super) const DENTRY_RESERVED_SIZE: usize = 3;

/// Default number of addresses reserved for inline xattrs.
pub(super) const DEFAULT_INLINE_XATTR_ADDRS: usize = 50;

/// Addresses reserved before inline data or inline dentries.
pub(super) const DEF_INLINE_RESERVED_SIZE: usize = 1;

/// Offset of the address array inside an inode node.
pub(super) const INODE_ADDR_OFFSET: usize = 360;

/// Offset of the node id array inside an inode node.
pub(super) const INODE_NID_OFFSET: usize = 4052;

/// Offset of the node footer inside any node block.
pub(super) const NODE_FOOTER_OFFSET: usize = 4072;

/// Offset of the journal area inside a summary block.
pub(super) const SUMMARY_JOURNAL_OFFSET: usize = 3584;

/// Offset of the footer inside a summary block.
pub(super) const SUMMARY_FOOTER_OFFSET: usize = 4091;

/// Summary block type of node summaries.
pub(super) const SUM_TYPE_NODE: u8 = 1;

/// Inode flag: the inode reserves space for inline xattrs.
pub(super) const F2FS_INLINE_XATTR: u8 = 0x01;

/// Inode flag: the inode stores file data inline.
pub(super) const F2FS_INLINE_DATA: u8 = 0x02;

/// Inode flag: the inode stores directory entries inline.
pub(super) const F2FS_INLINE_DENTRY: u8 = 0x04;

/// Inode flag: inline data actually exists.
pub(super) const F2FS_DATA_EXIST: u8 = 0x08;

/// Inode flag: `.` and `..` are implicit in an inline directory.
pub(super) const F2FS_INLINE_DOTS: u8 = 0x10;

/// Inode flag: the inode carries the extra attribute area.
pub(super) const F2FS_EXTRA_ATTR: u8 = 0x20;

/// Superblock feature: on-disk inodes carry the extra attribute area.
pub(super) const F2FS_FEATURE_EXTRA_ATTR: u32 = 0x0000_0008;

/// Superblock feature: inline xattr size is stored in the inode.
pub(super) const F2FS_FEATURE_FLEXIBLE_INLINE_XATTR: u32 = 0x0000_0040;

/// Superblock feature: inode checksums are enabled.
pub(super) const F2FS_FEATURE_INODE_CHKSUM: u32 = 0x0000_0020;

/// Superblock feature: inode creation times are recorded.
pub(super) const F2FS_FEATURE_INODE_CRTIME: u32 = 0x0000_0100;

/// Superblock feature: quota inodes are present.
pub(super) const F2FS_FEATURE_QUOTA_INO: u32 = 0x0000_0080;

/// Superblock feature: project quota is enabled.
pub(super) const F2FS_FEATURE_PRJQUOTA: u32 = 0x0000_0010;

/// Superblock feature: the volume is read-only.
pub(super) const F2FS_FEATURE_RO: u32 = 0x0000_4000;

/// Superblock feature: the superblock has a checksum.
pub(super) const F2FS_FEATURE_SB_CHKSUM: u32 = 0x0000_0800;

/// Superblock feature: the lost+found directory is present.
pub(super) const F2FS_FEATURE_LOST_FOUND: u32 = 0x0000_0200;

/// Superblock feature: the volume is zoned block device based.
pub(super) const F2FS_FEATURE_BLKZONED: u32 = 0x0000_0002;

/// Superblock feature: encryption is enabled.
pub(super) const F2FS_FEATURE_ENCRYPT: u32 = 0x0000_0001;

/// Superblock feature: atomic writes are enabled.
pub(super) const F2FS_FEATURE_ATOMIC_WRITE: u32 = 0x0000_0004;

/// Superblock feature: fs-verity is enabled.
pub(super) const F2FS_FEATURE_VERITY: u32 = 0x0000_0400;

/// Superblock feature: case-insensitive names are enabled.
pub(super) const F2FS_FEATURE_CASEFOLD: u32 = 0x0000_1000;

/// Superblock feature: compression is enabled.
pub(super) const F2FS_FEATURE_COMPRESSION: u32 = 0x0000_2000;

/// Superblock feature: multiple devices are allowed.
pub(super) const F2FS_FEATURE_DEVICE_ALIAS: u32 = 0x0000_8000;

/// Features that this read-only implementation can safely ignore.
pub(super) const SUPPORTED_FEATURES: u32 = F2FS_FEATURE_EXTRA_ATTR
    | F2FS_FEATURE_FLEXIBLE_INLINE_XATTR
    | F2FS_FEATURE_INODE_CHKSUM
    | F2FS_FEATURE_INODE_CRTIME
    | F2FS_FEATURE_PRJQUOTA
    | F2FS_FEATURE_QUOTA_INO
    | F2FS_FEATURE_RO
    | F2FS_FEATURE_SB_CHKSUM
    | F2FS_FEATURE_LOST_FOUND;

/// Features that fundamentally change the on-disk interpretation and are
/// therefore rejected at mount time.
pub(super) const UNSUPPORTED_FEATURES: u32 = F2FS_FEATURE_ENCRYPT
    | F2FS_FEATURE_BLKZONED
    | F2FS_FEATURE_ATOMIC_WRITE
    | F2FS_FEATURE_VERITY
    | F2FS_FEATURE_CASEFOLD
    | F2FS_FEATURE_COMPRESSION
    | F2FS_FEATURE_DEVICE_ALIAS;

/// Reads a little-endian `u16` from `buf` at `offset`.
pub(super) fn read_u16(buf: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([buf[offset], buf[offset + 1]])
}

/// Reads a little-endian `u32` from `buf` at `offset`.
pub(super) fn read_u32(buf: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
    ])
}

/// Reads a little-endian `u64` from `buf` at `offset`.
pub(super) fn read_u64(buf: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
        buf[offset + 4],
        buf[offset + 5],
        buf[offset + 6],
        buf[offset + 7],
    ])
}

/// Tests a bit of an F2FS on-disk bitmap, which is MSB-first.
pub(super) fn test_bit_be(bitmap: &[u8], bit: usize) -> bool {
    bitmap[bit / 8] & (1 << (7 - (bit % 8))) != 0
}

/// Tests a bit of a directory entry bitmap, which is LSB-first.
pub(super) fn test_bit_le(bitmap: &[u8], bit: usize) -> bool {
    bitmap[bit / 8] & (1 << (bit % 8)) != 0
}

/// The parsed F2FS superblock.
///
/// Some fields are kept for completeness and future write support, even
/// though the read-only driver does not consume them yet.
#[expect(dead_code)]
#[derive(Clone, Debug)]
pub(super) struct SuperBlock {
    pub log_blocks_per_seg: u32,
    pub segs_per_sec: u32,
    pub block_count: u64,
    pub section_count: u32,
    pub segment_count: u32,
    pub segment_count_ckpt: u32,
    pub segment_count_sit: u32,
    pub segment_count_nat: u32,
    pub segment_count_ssa: u32,
    pub segment_count_main: u32,
    pub cp_blkaddr: u32,
    pub sit_blkaddr: u32,
    pub nat_blkaddr: u32,
    pub ssa_blkaddr: u32,
    pub main_blkaddr: u32,
    pub root_ino: u32,
    pub node_ino: u32,
    pub meta_ino: u32,
    pub cp_payload: u32,
    pub feature: u32,
}

impl SuperBlock {
    /// Parses the superblock from the first block of the device.
    pub(super) fn parse(block0: &[u8]) -> Result<Self> {
        let raw = &block0[F2FS_SUPER_OFFSET..];
        if read_u32(raw, 0) != F2FS_SUPER_MAGIC {
            return_errno_with_message!(Errno::EINVAL, "not an F2FS filesystem");
        }

        let log_sectorsize = read_u32(raw, 8);
        let log_sectors_per_block = read_u32(raw, 12);
        let log_blocksize = read_u32(raw, 16);
        if log_blocksize != F2FS_LOG_BLKSIZE || log_sectorsize != 9 || log_sectors_per_block != 3 {
            return_errno_with_message!(Errno::EINVAL, "unsupported F2FS sector or block size");
        }

        let feature = read_u32(raw, 2180);
        if feature & UNSUPPORTED_FEATURES != 0 {
            return_errno_with_message!(
                Errno::EOPNOTSUPP,
                "the F2FS volume uses features that are not supported"
            );
        }
        if feature & !(SUPPORTED_FEATURES | UNSUPPORTED_FEATURES) != 0 {
            return_errno_with_message!(Errno::EOPNOTSUPP, "the F2FS volume uses unknown features");
        }

        let super_block = Self {
            log_blocks_per_seg: read_u32(raw, 20),
            segs_per_sec: read_u32(raw, 24),
            block_count: read_u64(raw, 36),
            section_count: read_u32(raw, 44),
            segment_count: read_u32(raw, 48),
            segment_count_ckpt: read_u32(raw, 52),
            segment_count_sit: read_u32(raw, 56),
            segment_count_nat: read_u32(raw, 60),
            segment_count_ssa: read_u32(raw, 64),
            segment_count_main: read_u32(raw, 68),
            // The addresses at offsets 72 (segment0_blkaddr) and 76 (cp_blkaddr)
            // are both needed; `segment0_blkaddr` is only used for sanity checks.
            cp_blkaddr: read_u32(raw, 76),
            sit_blkaddr: read_u32(raw, 80),
            nat_blkaddr: read_u32(raw, 84),
            ssa_blkaddr: read_u32(raw, 88),
            main_blkaddr: read_u32(raw, 92),
            root_ino: read_u32(raw, 96),
            node_ino: read_u32(raw, 100),
            meta_ino: read_u32(raw, 104),
            cp_payload: read_u32(raw, 1664),
            feature,
        };

        let segment0_blkaddr = read_u32(raw, 72);
        if super_block.log_blocks_per_seg < 9 || super_block.log_blocks_per_seg > 12 {
            return_errno_with_message!(Errno::EINVAL, "invalid F2FS segment size");
        }
        if super_block.block_count == 0
            || super_block.segment_count == 0
            || super_block.segment_count_main == 0
        {
            return_errno_with_message!(Errno::EINVAL, "invalid F2FS segment counts");
        }
        if super_block.cp_blkaddr < segment0_blkaddr
            || super_block.sit_blkaddr <= super_block.cp_blkaddr
            || super_block.nat_blkaddr <= super_block.sit_blkaddr
            || super_block.ssa_blkaddr <= super_block.nat_blkaddr
            || super_block.main_blkaddr <= super_block.ssa_blkaddr
        {
            return_errno_with_message!(Errno::EINVAL, "invalid F2FS area layout");
        }

        Ok(super_block)
    }

    /// Returns the number of blocks in a segment.
    pub(super) fn blocks_per_seg(&self) -> u32 {
        1 << self.log_blocks_per_seg
    }
}

/// One NAT entry.
#[derive(Clone, Copy, Debug)]
pub(super) struct NatEntry {
    pub ino: u32,
    pub block_addr: u32,
}

/// The parsed checkpoint.
///
/// SIT and flag fields are retained for future write support.
#[expect(dead_code)]
#[derive(Clone, Debug)]
pub(super) struct Checkpoint {
    pub version: u64,
    pub user_block_count: u64,
    pub valid_block_count: u64,
    pub valid_node_count: u32,
    pub valid_inode_count: u32,
    pub free_segment_count: u32,
    pub ckpt_flags: u32,
    pub cp_pack_total_block_count: u32,
    pub cp_pack_start_sum: u32,
    pub cur_node_segno: [u32; 8],
    pub cur_data_segno: [u32; 8],
    pub sit_ver_bitmap_bytesize: u32,
    pub nat_ver_bitmap_bytesize: u32,
    pub next_free_nid: u32,
    pub sit_bitmap: Vec<u8>,
    pub nat_bitmap: Vec<u8>,
}

impl Checkpoint {
    /// Parses a checkpoint block.
    pub(super) fn parse(block: &[u8]) -> Result<Self> {
        let version = read_u64(block, 0);
        let sit_ver_bitmap_bytesize = read_u32(block, 156);
        let nat_ver_bitmap_bytesize = read_u32(block, 160);
        let cp_pack_total_block_count = read_u32(block, 136);

        if cp_pack_total_block_count == 0 || cp_pack_total_block_count as usize > F2FS_BLKSIZE {
            return_errno_with_message!(Errno::EINVAL, "invalid F2FS checkpoint pack size");
        }
        let bitmap_bytes = sit_ver_bitmap_bytesize as usize + nat_ver_bitmap_bytesize as usize;
        if bitmap_bytes > F2FS_BLKSIZE - 192 {
            return_errno_with_message!(Errno::EINVAL, "invalid F2FS checkpoint bitmaps");
        }

        let sit_bitmap = block[192..192 + sit_ver_bitmap_bytesize as usize].to_vec();
        let nat_bitmap = block[192 + sit_ver_bitmap_bytesize as usize
            ..192 + sit_ver_bitmap_bytesize as usize + nat_ver_bitmap_bytesize as usize]
            .to_vec();

        Ok(Self {
            version,
            user_block_count: read_u64(block, 8),
            valid_block_count: read_u64(block, 16),
            valid_node_count: read_u32(block, 144),
            valid_inode_count: read_u32(block, 148),
            free_segment_count: read_u32(block, 32),
            ckpt_flags: read_u32(block, 132),
            cp_pack_total_block_count,
            cp_pack_start_sum: read_u32(block, 140),
            cur_node_segno: core::array::from_fn(|i| read_u32(block, 36 + i * 4)),
            cur_data_segno: core::array::from_fn(|i| read_u32(block, 84 + i * 4)),
            sit_ver_bitmap_bytesize,
            nat_ver_bitmap_bytesize,
            next_free_nid: read_u32(block, 152),
            sit_bitmap,
            nat_bitmap,
        })
    }
}

/// Parses a NAT entry from a NAT block.
pub(super) fn parse_nat_entry(block: &[u8], index: usize) -> NatEntry {
    let offset = index * NAT_ENTRY_SIZE;
    NatEntry {
        ino: read_u32(block, offset + 1),
        block_addr: read_u32(block, offset + 5),
    }
}

/// Computes the F2FS directory entry hash (a TEA-based hash copied from ext3).
pub(super) fn dentry_hash(name: &[u8]) -> u32 {
    const DELTA: u32 = 0x9e37_79b9;

    fn tea_transform(buf: &mut [u32; 4], input: &[u32; 4]) {
        let mut sum = 0u32;
        let mut b0 = buf[0];
        let mut b1 = buf[1];
        let (a, b, c, d) = (input[0], input[1], input[2], input[3]);
        for _ in 0..16 {
            sum = sum.wrapping_add(DELTA);
            b0 = b0.wrapping_add(
                (b1.wrapping_shl(4).wrapping_add(a))
                    ^ b1.wrapping_add(sum)
                    ^ (b1.wrapping_shr(5).wrapping_add(b)),
            );
            b1 = b1.wrapping_add(
                (b0.wrapping_shl(4).wrapping_add(c))
                    ^ b0.wrapping_add(sum)
                    ^ (b0.wrapping_shr(5).wrapping_add(d)),
            );
        }
        buf[0] = buf[0].wrapping_add(b0);
        buf[1] = buf[1].wrapping_add(b1);
    }

    fn str2hashbuf(msg: &[u8]) -> [u32; 4] {
        let len = msg.len();
        let pad = (len as u32) | ((len as u32) << 8);
        let pad = pad | (pad << 16);
        let mut out = [pad; 4];
        let mut val = pad;
        let mut num: i32 = 4;
        let mut index = 0;
        let limit = len.min(16);
        for (i, byte) in msg[..limit].iter().enumerate() {
            if i % 4 == 0 {
                val = pad;
            }
            val = (*byte as u32).wrapping_add(val << 8);
            if i % 4 == 3 {
                out[index] = val;
                index += 1;
                num -= 1;
                val = pad;
            }
        }
        num -= 1;
        if num >= 0 && index < 4 {
            out[index] = val;
            index += 1;
        }
        num -= 1;
        while num >= 0 && index < 4 {
            out[index] = pad;
            index += 1;
            num -= 1;
        }
        out
    }

    let mut buf = [0x6745_2301u32, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    let mut remaining = name;
    let mut total = name.len();
    loop {
        let input = str2hashbuf(remaining);
        tea_transform(&mut buf, &input);
        if total <= 16 {
            break;
        }
        remaining = &remaining[16..];
        total -= 16;
    }
    buf[0]
}

/// Converts an F2FS on-disk file type to the VFS inode type.
pub(super) fn inode_type_from_dentry(file_type: u8) -> crate::fs::file::InodeType {
    use crate::fs::file::InodeType;

    match file_type {
        1 => InodeType::File,
        2 => InodeType::Dir,
        3 => InodeType::CharDevice,
        4 => InodeType::BlockDevice,
        5 => InodeType::NamedPipe,
        6 => InodeType::Socket,
        7 => InodeType::SymLink,
        _ => InodeType::Unknown,
    }
}

/// Converts a POSIX mode to the VFS inode type.
pub(super) fn inode_type_from_mode(mode: u16) -> crate::fs::file::InodeType {
    use crate::fs::file::InodeType;

    match mode & 0o170000 {
        0o010000 => InodeType::NamedPipe,
        0o020000 => InodeType::CharDevice,
        0o040000 => InodeType::Dir,
        0o060000 => InodeType::BlockDevice,
        0o100000 => InodeType::File,
        0o120000 => InodeType::SymLink,
        0o140000 => InodeType::Socket,
        _ => InodeType::Unknown,
    }
}

/// Computes the Linux `crc32_le` checksum with an explicit seed.
pub(super) fn crc32_le(mut crc: u32, data: &[u8]) -> u32 {
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    crc
}
