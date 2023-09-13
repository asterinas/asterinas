//! A safe Rust Ext2 filesystem.
#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), forbid(unsafe_code))]
#![allow(dead_code)]
#![feature(int_roundings)]
#![feature(trait_upcasting)]

extern crate alloc;

extern crate log;

pub use fs::Ext2;
pub use inode::{Ext2Inode, FilePerm, FileType};
pub use super_block::Ext2SuperBlock;

pub mod error;
pub mod traits;

mod bitmap;
mod block_group;
mod dir;
mod fs;
mod inode;
mod prelude;
mod super_block;
mod utils;

#[cfg(test)]
mod test;

#[cfg(test)]
#[macro_use]
extern crate lazy_static;
