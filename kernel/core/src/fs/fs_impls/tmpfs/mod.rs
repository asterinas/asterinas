// SPDX-License-Identifier: MPL-2.0

//! Temporary file system (tmpfs) based on ramfs.

use fs::TmpFsType;
pub(super) use fs::{TmpFs, default_max_blocks, default_max_inodes};

mod fs;

pub(super) const TMPFS_MAGIC: u64 = 0x0102_1994;

pub(super) fn init() {
    crate::fs::vfs::registry::register(&TmpFsType).unwrap();
}
