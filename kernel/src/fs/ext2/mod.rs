// SPDX-License-Identifier: MPL-2.0

//! A safe Rust Ext2 filesystem.
//!
//! The Second Extended File System(Ext2) is a major rewrite of the Ext filesystem.
//! It is the predominant filesystem in use by Linux from the early 1990s to the early 2000s.
//! The structures of Ext3 and Ext4 are based on Ext2 and add some additional options
//! such as journaling.
//!
//! The features of this version of Ext2 are as follows:
//! 1. No unsafe Rust. The filesystem is written is Rust without any unsafe code,
//!    ensuring that there are no memory safety issues in the code.
//! 2. Deep integration with PageCache. The data and metadata of the filesystem are
//!    stored in PageCache, which accelerates the performance of data access.
//! 3. Compatible with queue-based block device. The filesystem can submits multiple
//!    BIO requests to be block device at once, thereby enhancing I/O performance.
//!
//! # Example
//!
//! ```no_run
//! // Opens an Ext2 from the block device.
//! let ext2 = Ext2::open(block_device)?;
//! // Lookup the root inode.
//! let root = ext2.root_inode()?;
//! // Create a file inside root directory.
//! let file = root.create("file", InodeType::File, FilePerm::from_bits_truncate(0o666))?;
//! // Write data into the file.
//! const WRITE_DATA: &[u8] = b"Hello, World";
//! let len = file.write_at(0, WRITE_DATA)?;
//! assert!(len == WRITE_DATA.len());
//! ```
//!
//! # Limitation
//!
//! Here we summarizes the features that need to be implemented in the future.
//! 1. Supports merging small read/write operations.
//! 2. Handles the intermediate failure status correctly.

pub use fs::Ext2;
pub use inode::{FilePerm, Inode};
pub use super_block::{SuperBlock, MAGIC_NUM};

mod block_group;
mod block_ptr;
mod dir;
mod fs;
mod impl_for_vfs;
mod indirect_block_cache;
mod inode;
mod prelude;
mod super_block;
mod utils;
