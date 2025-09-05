// SPDX-License-Identifier: MPL-2.0

//! This module offers `/proc/uptime` file support, which provides
//! information about the system uptime and idle time.
//!
//! Reference: <https://man7.org/linux/man-pages/man5/proc_uptime.5.html>

use alloc::format;

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::Inode,
    },
    prelude::*,
    time::cpu_stat,
};

pub struct UptimeFileOps;

impl UptimeFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self).parent(parent).build().unwrap()
    }
    pub fn collect_uptime() -> String {
        let uptime = aster_time::read_monotonic_time().as_secs_f32();
        let cpustat = cpu_stat::CpuStatManager::get();
        let idle_time = cpustat.get_global().idle.as_duration().as_secs_f32();
        format!("{:.2}  {:.2}", uptime, idle_time)
    }
}
impl FileOps for UptimeFileOps {
    /// Retrieve the data for `/proc/uptime`.
    fn data(&self) -> Result<Vec<u8>> {
        let output = Self::collect_uptime();
        Ok(output.into_bytes())
    }
}
