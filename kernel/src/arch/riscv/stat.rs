// SPDX-License-Identifier: MPL-2.0

use crate::{fs::utils::Metadata, prelude::*, time::timespec_t};

#[derive(Debug, Clone, Copy, Pod, Default)]
#[repr(C)]
pub struct Stat {
    /// ID of device containing file
    st_dev: u64,
    /// Inode number
    st_ino: u64,
    /// File type and mode
    st_mode: u32,
    /// Number of hard links
    st_nlink: u32,
    /// User ID of owner
    st_uid: u32,
    /// Group ID of owner
    st_gid: u32,
    /// Device ID (if special file)
    st_rdev: u64,
    /// Padding bytes
    __pad0: u64,
    /// Total size, in bytes
    st_size: isize,
    /// Block size for filesystem I/O
    st_blksize: isize,
    /// Number of 512-byte blocks allocated
    st_blocks: isize,
    /// Time of last access
    st_atime: timespec_t,
    /// Time of last modification
    st_mtime: timespec_t,
    /// Time of last status change
    st_ctime: timespec_t,
    /// Unused field
    __unused: [u32; 2],
}

impl From<Metadata> for Stat {
    fn from(info: Metadata) -> Self {
        Self {
            st_dev: info.dev,
            st_ino: info.ino,
            st_nlink: info.nlinks as u32,
            st_mode: info.type_ as u32 | info.mode.bits() as u32,
            st_uid: info.uid.into(),
            st_gid: info.gid.into(),
            __pad0: 0,
            st_rdev: info.rdev,
            st_size: info.size as isize,
            st_blksize: info.blk_size as isize,
            st_blocks: (info.blocks * (info.blk_size / 512)) as isize, // Number of 512B blocks
            st_atime: info.atime.into(),
            st_mtime: info.mtime.into(),
            st_ctime: info.ctime.into(),
            __unused: [0; 2],
        }
    }
}
