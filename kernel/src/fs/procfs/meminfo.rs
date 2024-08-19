// SPDX-License-Identifier: MPL-2.0

//! This module offers `/proc/meminfo` file support, which tells the user space
//! about the memory statistics in the entire system. The definition of the
//! fields are similar to that of Linux's but there exist differences.
//!
//! Reference: <https://man7.org/linux/man-pages/man5/proc_meminfo.5.html>

use alloc::format;

use ostd::mm::stat;

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::Inode,
    },
    prelude::*,
};

/// Represents the inode at `/proc/meminfo`.
pub struct MemInfoFileOps;

impl MemInfoFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self).parent(parent).build().unwrap()
    }
}

/// Total memory in the entire system in bytes.
fn mem_total() -> usize {
    stat::mem_total()
}

/// An estimation of how much memory is available for starting new
/// applications, without disk operations.
fn mem_available() -> usize {
    stat::mem_available()
}

impl FileOps for MemInfoFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let total = mem_total();
        let available = mem_available();
        let output = format!("MemTotal:\t{}\nMemAvailable:\t{}\n", total, available);
        Ok(output.into_bytes())
    }
}
