// SPDX-License-Identifier: MPL-2.0

//! Ext4 filesystem support.
//!
//! This module implements read-only Ext4 filesystem support.

pub use fs::Ext4;
pub use inode::{FilePerm, Inode as Ext4Inode};
pub use super_block::MAGIC_NUM;

use fs::Ext4Type;

mod block_group;
mod dir;
mod extent;
mod fs;
mod impl_for_vfs;
mod inode;
mod prelude;
mod super_block;

pub(super) fn init() {
    crate::fs::vfs::registry::register(&Ext4Type).unwrap();
}
