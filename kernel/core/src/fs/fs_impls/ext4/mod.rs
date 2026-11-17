// SPDX-License-Identifier: MPL-2.0

//! Shared EXT2/EXT4 filesystem implementation.
//!
//! [`Ext4`] represents a mounted filesystem; [`Inode`] provides file, directory,
//! symlink, and extended-attribute operations through `impl_for_vfs`.
//! `block_group` manages local allocation and inode tables, while `super_block`
//! validates filesystem geometry and features.
//!
//! Supports EXT2 and non-journaled EXT4 with 4 KiB blocks and writable extents.
//! Journaling, inline data, metadata checksums, 64-bit block addresses, and
//! indexed directories are unsupported.
//!
//! # References
//!
//! - <https://www.kernel.org/doc/html/latest/filesystems/ext2.html>
//! - <https://www.nongnu.org/ext2-doc/ext2.html>

// Set this module's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "ext2: "
    };
}

pub(crate) use fs::Ext4;
pub(crate) use inode::{FilePerm, Inode};

pub(in crate::fs) use self::fs_type::{EXT2_TYPE, EXT4_TYPE};
use crate::fs::vfs::registry;

mod block_group;
mod fs;
mod fs_type;
mod impl_for_vfs;
mod inode;
mod prelude;
mod super_block;
mod utils;
mod xattr;

#[cfg(ktest)]
mod test_utils;

/// Registers the ext2 and ext4 filesystem types with the VFS registry.
pub(super) fn init() {
    registry::register(&EXT4_TYPE).unwrap();
    registry::register(&EXT2_TYPE).unwrap();
}
