// SPDX-License-Identifier: MPL-2.0

//! Ramfs based on PageCache

pub use fs::RamFS;

mod fs;

const RAMFS_MAGIC: u64 = 0x0102_1994;
const BLOCK_SIZE: usize = 4096;
const ROOT_INO: u64 = 1;
const NAME_MAX: usize = 255;
