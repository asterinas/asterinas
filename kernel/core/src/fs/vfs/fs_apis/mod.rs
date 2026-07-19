// SPDX-License-Identifier: MPL-2.0

//! Core VFS abstractions.
//!
//! This module defines the fundamental interfaces that file systems should implement.

pub mod file_system;
pub mod inode;
pub mod inode_ext;
pub mod registry;
pub mod xattr;

pub(super) fn init() {
    registry::init();
}
