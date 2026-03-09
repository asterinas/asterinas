// SPDX-License-Identifier: MPL-2.0

//! Virtual File System (VFS) layer.
//!
//! The VFS provides a unified abstraction over different file system implementations,
//! serving as the bridge between system calls and concrete file systems.

pub mod fs_apis;
pub mod notify;
pub mod page_cache;
pub mod path;
pub mod range_lock;

// Re-export commonly used abstractions from fs_apis
pub use fs_apis::{file_system, inode, inode_ext, registry, xattr};

pub(super) fn init() {
    fs_apis::init();
    path::init();
}
