// SPDX-License-Identifier: MPL-2.0

//! This module offers `/proc/uptime` file support, which provides
//! information about the system uptime and idle time.
//!
//! Reference: <https://man7.org/linux/man-pages/man5/proc_uptime.5.html>

use alloc::format;

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{Inode, InodeMode},
    },
    prelude::*,
    time::cpu_time_stats::CpuTimeStatsManager,
};

/// Represents the inode at `/proc/uptime`.  
pub struct UptimeFileOps;

impl UptimeFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self, InodeMode::from_bits_truncate(0o444))
            .parent(parent)
            .build()
            .unwrap()
    }

    pub fn collect_uptime() -> String {
        let uptime = aster_time::read_monotonic_time().as_secs_f32();
        let cpustat = CpuTimeStatsManager::singleton();
        let idle_time = cpustat
            .collect_stats_on_all_cpus()
            .idle
            .as_duration()
            .as_secs_f32();
        format!("{:.2} {:.2}\n", uptime, idle_time)
    }
}

impl FileOps for UptimeFileOps {
    /// Retrieve the data for `/proc/uptime`.
    fn data(&self) -> Result<Vec<u8>> {
        let output = Self::collect_uptime();
        Ok(output.into_bytes())
    }
}
