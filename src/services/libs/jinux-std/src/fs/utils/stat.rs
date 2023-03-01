#![allow(non_camel_case_types)]

use super::{Metadata, Timespec};
use crate::prelude::*;

pub const S_IFMT: u32 = 0o170000;
pub const S_IFCHR: u32 = 0o020000;
pub const S_IFDIR: u32 = 0o040000;
pub const S_IFREG: u32 = 0o100000;
pub const S_IFLNK: u32 = 0o120000;

/// File Stat
#[derive(Debug, Clone, Copy, Pod, Default)]
#[repr(C)]
pub struct Stat {
    /// ID of device containing file
    st_dev: usize,
    /// Inode number
    st_ino: usize,
    /// Number of hard links
    st_nlink: usize,
    /// File type and mode
    st_mode: u32,
    /// User ID of owner
    st_uid: u32,
    /// Group ID of owner
    st_gid: u32,
    /// Padding bytes
    __pad0: u32,
    /// Device ID (if special file)
    st_rdev: usize,
    /// Total size, in bytes
    st_size: isize,
    /// Block size for filesystem I/O
    st_blksize: isize,
    /// Number of 512-byte blocks allocated
    st_blocks: isize,
    /// Time of last access
    st_atime: Timespec,
    /// Time of last modification
    st_mtime: Timespec,
    /// Time of last status change
    st_ctime: Timespec,
    /// Unused field
    __unused: [i64; 3],
}

impl From<Metadata> for Stat {
    fn from(info: Metadata) -> Self {
        Self {
            st_dev: info.dev,
            st_ino: info.ino,
            st_nlink: info.nlinks,
            st_mode: info.type_ as u32 | info.mode.bits() as u32,
            st_uid: info.uid as u32,
            st_gid: info.gid as u32,
            __pad0: 0,
            st_rdev: 0,
            st_size: info.size as isize,
            st_blksize: info.blk_size as isize,
            st_blocks: info.blocks as isize,
            st_atime: info.atime,
            st_mtime: info.mtime,
            st_ctime: info.ctime,
            __unused: [0; 3],
        }
    }
}
