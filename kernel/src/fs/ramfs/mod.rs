// SPDX-License-Identifier: MPL-2.0

//! Ramfs based on PageCache

use alloc::sync::Arc;

pub use fs::{new_detached_inode, RamFS};

use crate::fs::ramfs::fs::RamFsType;

mod fs;
mod xattr;

const RAMFS_MAGIC: u64 = 0x0102_1994;
const BLOCK_SIZE: usize = 4096;
const ROOT_INO: u64 = 1;
const NAME_MAX: usize = 255;

pub(super) fn init() {
    let ramfs_type = Arc::new(RamFsType);
    super::registry::register(ramfs_type).unwrap();
}
