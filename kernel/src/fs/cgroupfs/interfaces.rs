// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use crate::{
    fs::utils::{FileSystem, Inode},
    prelude::*,
};

pub const BLOCK_SIZE_KERNFS: usize = 1024;

/// Provides interfaces for reading and writing to a pseudo file.
pub trait DataProvider: Any + Sync + Send {
    /// Reads data from the file at the given offset.
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize>;
    /// Writes data to the file at the given offset.
    fn write_at(&mut self, offset: usize, reader: &mut VmReader) -> Result<usize>;
}

/// Provides different abstractions for cgroup-specific filesystem requirements.
pub trait CgroupExt: Send + Sync {
    /// Creates a new node.
    fn on_create(&self, name: &str, node: Arc<dyn Inode>) -> Result<()>;
    /// Removes a node.
    fn on_remove(&self, name: &str) -> Result<()>;
}
