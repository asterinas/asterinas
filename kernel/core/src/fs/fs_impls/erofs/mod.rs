// SPDX-License-Identifier: MPL-2.0

//! EROFS filesystem implementation.

use crate::fs::vfs::registry;

mod compressed;
mod decompressor;
mod fs;
mod fs_type;
mod impl_for_vfs;
mod inode;
mod io;
mod meta;
mod ondisk;

/// Registers the EROFS filesystem type with the VFS registry.
pub(super) fn init() {
    registry::register(&fs_type::EROFS_TYPE).unwrap();
}
