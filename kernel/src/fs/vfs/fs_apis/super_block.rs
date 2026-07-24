// SPDX-License-Identifier: MPL-2.0

//! VFS superblock objects shared by mounts of the same filesystem instance.

use device_id::DeviceId;

use super::file_system::{FileSystem, FsStats};
use crate::{
    prelude::*,
    thread::work_queue::{self, WorkPriority},
};

/// A live filesystem instance shared by one or more mounts.
#[derive(Debug)]
pub struct SuperBlock {
    fs: Arc<dyn FileSystem>,
    magic: u64,
    block_size: usize,
    name_max: usize,
    container_device_id: DeviceId,
}

impl SuperBlock {
    /// Creates a superblock for a filesystem implementation.
    pub fn new(
        fs: Arc<dyn FileSystem>,
        magic: u64,
        block_size: usize,
        name_max: usize,
        container_device_id: DeviceId,
    ) -> Arc<Self> {
        Arc::new(Self {
            fs,
            magic,
            block_size,
            name_max,
            container_device_id,
        })
    }

    /// Returns the filesystem implementation owned by this superblock.
    pub fn fs(&self) -> &Arc<dyn FileSystem> {
        &self.fs
    }

    /// Returns the current filesystem statistics.
    pub fn stats(&self) -> FsStats {
        self.fs.stats()
    }

    pub fn magic(&self) -> u64 {
        self.magic
    }

    pub fn block_size(&self) -> usize {
        self.block_size
    }

    pub fn name_max(&self) -> usize {
        self.name_max
    }

    pub fn container_device_id(&self) -> DeviceId {
        self.container_device_id
    }
}

impl Drop for SuperBlock {
    fn drop(&mut self) {
        let fs = self.fs.clone();
        let sync_fn = move || {
            if let Err(err) = fs.sync() {
                error!(
                    "failed to sync filesystem {} during final cleanup: {:?}",
                    fs.name(),
                    err
                );
            }
        };

        if work_queue::is_initialized() {
            work_queue::submit_work_func(sync_fn, WorkPriority::Normal);
        } else {
            // The initial mount namespace is created before the work queues.
            sync_fn();
        }
    }
}
