// SPDX-License-Identifier: MPL-2.0

//! Core VFS abstractions.
//!
//! This module defines the fundamental interfaces that file systems should implement.

pub(crate) mod file_system;
pub(crate) mod inode;
pub(crate) mod inode_ext;
pub(crate) mod registry;
pub(crate) mod xattr;

pub(super) fn init() {
    registry::init();
}
