// SPDX-License-Identifier: MPL-2.0

//! This module offers `/proc/cpuinfo` file support, which provides
//! information about the CPU architecture, cores, and other details.
//!
//! Reference: <https://man7.org/linux/man-pages/man5/proc_cpuinfo.5.html>

use ostd::cpu::num_cpus;

use crate::{
    arch::cpu::CpuInfo,
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::Inode,
    },
    prelude::*,
};

/// Represents the inode at `/proc/cpuinfo`.
pub struct CpuInfoFileOps;

impl CpuInfoFileOps {
    /// Create a new inode for `/proc/cpuinfo`.
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self).parent(parent).build().unwrap()
    }

    /// Collect and format CPU information for all cores.
    fn collect_cpu_info() -> String {
        let num_cpus = num_cpus() as u32;

        // Iterate over each core and collect CPU information
        (0..num_cpus)
            .map(|core_id| {
                let cpuinfo = CpuInfo::new(core_id);
                cpuinfo.collect_cpu_info()
            })
            .collect::<Vec<String>>()
            .join("\n\n")
    }
}

impl FileOps for CpuInfoFileOps {
    /// Retrieve the data for `/proc/cpuinfo`.
    fn data(&self) -> Result<Vec<u8>> {
        let output = Self::collect_cpu_info();
        Ok(output.into_bytes())
    }
}
