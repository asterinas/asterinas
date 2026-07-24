// SPDX-License-Identifier: MPL-2.0

//! VFS superblock objects shared by mounts of the same filesystem instance.

use super::file_system::FileSystem;
use crate::prelude::*;

/// A live filesystem instance shared by one or more mounts.
#[derive(Debug)]
pub struct SuperBlock {
    fs: Arc<dyn FileSystem>,
}

impl SuperBlock {
    /// Creates a superblock for a filesystem implementation.
    pub fn new(fs: Arc<dyn FileSystem>) -> Arc<Self> {
        Arc::new(Self { fs })
    }

    /// Returns the filesystem implementation owned by this superblock.
    pub fn fs(&self) -> &Arc<dyn FileSystem> {
        &self.fs
    }
}
