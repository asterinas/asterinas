// SPDX-License-Identifier: MPL-2.0

//! Ramfs based on PageCache

pub(crate) use fs::RamFs;
use fs::RamFsType;
pub(super) use fs::RamInode;

mod fs;
pub(crate) mod memfd;
mod xattr;

const RAMFS_MAGIC: u64 = 0x8584_58f6;
pub(super) const BLOCK_SIZE: usize = 4096;
/// The maximum file size representable by a signed offset.
pub(super) const MAX_FILE_SIZE: usize = isize::MAX as usize;
const ROOT_INO: u64 = 1;
pub(super) const NAME_MAX: usize = 255;

pub(super) fn init() {
    crate::fs::vfs::registry::register(&RamFsType).unwrap();
    memfd::init();
}
