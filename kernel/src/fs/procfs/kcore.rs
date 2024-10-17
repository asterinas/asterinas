// SPDX-License-Identifier: MPL-2.0

//! This module offers `/proc/kcore` file support, which provides a snapshot of
//! the system's memory for kernel debugging tools. The file does not occupy
//! real disk space and is dynamically generated when accessed.
//!
//! Reference: <https://man7.org/linux/man-pages/man5/proc.5.html>

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::Inode,
    },
    prelude::*,
};

pub struct KCoreFileOps;

impl KCoreFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self).parent(parent).build().unwrap()
    }
}

impl KCoreFileOps {
    fn read_memory_snapshot(&self) -> Vec<u8> {
        let dummy_data = vec![0u8; 64];
        dummy_data
    }
}

/// FIXME: We should not return all memory snapshot data at once.
/// Current implementation cannot satisfy the actual requirements.
/// Use the trait DataProvider to implement the actual logic later.
impl FileOps for KCoreFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        // For simplicity, let's simulate kcore data by outputting a static message
        // In practice, this would involve fetching and returning memory snapshot data.
        Ok(self.read_memory_snapshot())
    }
}
