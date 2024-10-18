// SPDX-License-Identifier: MPL-2.0

//! This module offers `/proc/loadavg` file support, which tells the user space
//! about the cpu load average for the last 1, 5, and 15 minutes.
//!
//! Reference: <https://www.man7.org/linux/man-pages/man5/proc_loadavg.5.html>

use alloc::format;

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::Inode,
    },
    prelude::*,
    process::posix_thread,
    sched::{self, loadavg::get_loadavg},
};

/// Represents the inode at `/proc/loadavg`.
pub struct LoadAvgFileOps;

impl LoadAvgFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self).parent(parent).build().unwrap()
    }
}

impl FileOps for LoadAvgFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let avg = get_loadavg();
        let (nr_queued, nr_running) = sched::nr_queued_and_running();

        let output = format!(
            "{:.2} {:.2} {:.2} {}/{} {}\n",
            avg[0],
            avg[1],
            avg[2],
            nr_running,
            nr_queued,
            posix_thread::last_tid(),
        );

        Ok(output.into_bytes())
    }
}
