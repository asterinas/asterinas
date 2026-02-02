// SPDX-License-Identifier: MPL-2.0

//! Core VFS abstractions.
//!
//! This module defines the fundamental interfaces that file systems should implement.

pub mod inode;
pub mod inode_ext;
pub mod registry;
pub mod super_block;
pub mod xattr;

pub(super) fn init() {
    registry::init();
}
