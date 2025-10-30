// SPDX-License-Identifier: MPL-2.0

//! Ramfs based on PageCache

use fs::RamFsType;
pub use fs::{new_detached_inode_in_memfd, RamFs, RamInode};

mod fs;
mod xattr;

const RAMFS_MAGIC: u64 = 0x8584_58f6;
const BLOCK_SIZE: usize = 4096;
const ROOT_INO: u64 = 1;
const NAME_MAX: usize = 255;

pub(super) fn init() {
    super::registry::register(&RamFsType).unwrap();
}
