// SPDX-License-Identifier: MPL-2.0

//! This module offers `/proc/meminfo` file support, which tells the user space
//! about the memory statistics in the entire system. The definition of the
//! fields are similar to that of Linux's but there exist differences.
//!
//! Reference: <https://man7.org/linux/man-pages/man5/proc_meminfo.5.html>

use alloc::format;

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{mkmod, Inode},
    },
    prelude::*,
};

/// Represents the inode at `/proc/meminfo`.
pub struct MemInfoFileOps;

impl MemInfoFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference:
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/meminfo.c#L178>
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/generic.c#L549-L550>
        ProcFileBuilder::new(Self, mkmod!(a+r))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for MemInfoFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        // The total amount of physical memory available to the system.
        let total = crate::vm::mem_total();
        // An estimation of how much memory is available for starting new
        // applications, without disk operations.
        let available = osdk_frame_allocator::load_total_free_size();

        // Convert the values to KiB.
        let total = total / 1024;
        let available = available / 1024;

        // Available memory should include both free memory and cached pages that can be
        // immediately evicted from main memory. Currently, no pages can be evicted when memory is
        // allocated, resulting in the two values being reported as the same.
        let output = format!(
            "MemTotal:\t{} kB\nMemFree:\t{} kB\nMemAvailable:\t{} kB\n",
            total, available, available
        );
        Ok(output.into_bytes())
    }
}
