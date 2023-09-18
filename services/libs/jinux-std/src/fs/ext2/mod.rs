//! A safe Rust Ext2 filesystem.

pub use dir::DirEntryReader;
pub use fs::Ext2;
pub use inode::{Ext2Inode, FilePerm, FileType};
pub use super_block::Ext2SuperBlock;

mod block_group;
mod dir;
mod error;
mod fs;
mod impl_for_vfs;
mod inode;
mod prelude;
mod super_block;
mod utils;
