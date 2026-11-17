// SPDX-License-Identifier: MPL-2.0

//! The POSIX message queue file system (`mqueuefs`).
//!
//! `mqueuefs` is a pseudo file system that exposes each POSIX message queue
//! as a regular file, typically mounted at `/dev/mqueue`.

// TODO: Remove this once the methods are implemented and the types are used.
#![allow(dead_code)]

mod fs;
mod inode;

pub(crate) use fs::MqueueFs;

/// Magic number of `mqueuefs`.
pub(super) const MQUEUE_MAGIC: u64 = 0x1980_0202;
/// Block size.
pub(super) const BLOCK_SIZE: usize = 4096;
/// Root inode number.
pub(super) const ROOT_INO: u64 = 1;

pub(super) fn init() {
    crate::fs::vfs::registry::register(&fs::MqueueFsType).unwrap();
}
