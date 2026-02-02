// SPDX-License-Identifier: MPL-2.0

//! Virtual File System (VFS) layer.
//!
//! The VFS provides a unified abstraction over different file system implementations,
//! serving as the bridge between system calls and concrete file systems.

pub mod core;
pub mod notify;
pub mod page_cache;
pub mod path;
pub mod range_lock;

// Re-export commonly used abstractions from core
pub use core::{inode, inode_ext, registry, super_block, xattr};

pub(super) fn init() {
    core::init();
    path::init();
}
