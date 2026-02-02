// SPDX-License-Identifier: MPL-2.0

//! Ramfs based on PageCache

pub use fs::RamFs;
use fs::RamFsType;

mod fs;
pub mod memfd;
mod xattr;

const RAMFS_MAGIC: u64 = 0x8584_58f6;
const BLOCK_SIZE: usize = 4096;
const ROOT_INO: u64 = 1;
const NAME_MAX: usize = 255;

pub(super) fn init() {
    crate::fs::vfs::registry::register(&RamFsType).unwrap();
}
