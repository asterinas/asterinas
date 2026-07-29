// SPDX-License-Identifier: MPL-2.0

//! Ext2 filesystem implementation, providing file I/O, directory operations,
//! symlinks, and extended attributes through the Asterinas VFS trait interfaces.
//!
//! This module is the entry point for ext2 support in Asterinas. A caller
//! registers `Ext2` as a filesystem type via `init`, after which the VFS
//! can mount ext2 volumes and operate on them through the standard
//! filesystem trait interfaces. Buffered I/O is delegated to the
//! `PageCache` subsystem; this module does not cache block data itself.
//!
//! The Second Extended File System (ext2) is a classic Linux filesystem
//! introduced in 1993 as a replacement for the original ext filesystem.
//! It was the default Linux filesystem throughout the 1990s and remains
//! the on-disk foundation for ext3 and ext4. This implementation covers
//! the base ext2 feature set; it does not include ext3/ext4 extensions
//! such as journaling, extents, or inline data.
//!
//! # On-disk layout
//!
//! An ext2 volume is divided into fixed-size block groups. Each block group
//! contains a block bitmap, an inode bitmap, an inode table, and data blocks.
//! The superblock and the block group descriptor table are stored in block
//! group 0 (with backup copies in select other groups). The `fs` module
//! manages the superblock and block group descriptor table, while
//! `block_group` tracks per-group allocation state.
//!
//! Individual files and directories are represented by inodes. The `inode`
//! module handles inode I/O, including data block mapping, directory entry
//! management, and symlink resolution. The `impl_for_vfs` module wires
//! these operations into the VFS trait interfaces so that the rest of the
//! kernel accesses ext2 through the common filesystem API.
//!
//! # Module structure
//!
//! | Module         | Responsibility                                       |
//! |----------------|------------------------------------------------------|
//! | `fs`           | Filesystem-level state: superblock, block groups     |
//! | `inode`        | Inode operations: file I/O, directories, symlinks    |
//! | `xattr`        | Extended attribute block management                  |
//! | `block_group`  | Block group descriptor and per-group allocation      |
//! | `super_block`  | On-disk superblock parsing and writeback             |
//! | `impl_for_vfs` | Wires ext2 types into the VFS trait interfaces       |
//! | `fs_type`      | `FsType` registration glue                           |
//! | `utils`        | Dirty tracking, sparse-super helpers, and time utils |
//! | `prelude`      | Common imports shared across submodules              |
//!
//! Directory entry layout helpers live under `inode::dir::dir_entry`,
//! next to the directory operations that use them.
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

pub use fs::Ext2;
pub use inode::{FilePerm, Inode};

use self::fs_type::Ext2Type;
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

/// Registers the ext2 filesystem type with the VFS registry.
pub(super) fn init() {
    registry::register(&Ext2Type).unwrap();
}
