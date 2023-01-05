//! Ramfs based on PageCache

pub use fs::RamFS;

mod fs;

const RAMFS_MAGIC: usize = 0x0102_1994;
const BLOCK_SIZE: usize = 4096;
const NAME_MAX: usize = 255;
const ROOT_INO: usize = 1;
