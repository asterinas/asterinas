// SPDX-License-Identifier: MPL-2.0

//! Ext4 filesystem implementation.
//!
//! The implementation supports extent-based and ext2-style indirect file I/O,
//! linear directories, special files, and extended attributes. Journaling,
//! metadata checksums, and indexed directories are not supported yet.

// Set this module's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "ext4: "
    };
}

pub use fs::Ext4;
pub use inode::{FilePerm, Inode};

use self::fs_type::Ext2Type;
use crate::fs::vfs::registry;

mod block_group;
mod feature;
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

/// Registers the ext2 filesystem type with the VFS registry. The ext4 type
/// name joins once the extent engine lands.
pub(super) fn init() {
    registry::register(&Ext2Type).unwrap();
}
