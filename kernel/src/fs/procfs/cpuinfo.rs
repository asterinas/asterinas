// SPDX-License-Identifier: MPL-2.0

//! This module offers `/proc/cpuinfo` file support, which provides
//! information about the CPU architecture, cores, and other details.
//!
//! Reference: <https://man7.org/linux/man-pages/man5/proc_cpuinfo.5.html>

use ostd::{
    cpu::{all_cpus, PinCurrentCpu},
    cpu_local,
    task::disable_preempt,
};
use spin::Once;

use crate::{
    arch::cpu::CpuInformation,
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::Inode,
    },
    prelude::*,
};

/// Represents the inode at `/proc/cpuinfo`.
pub struct CpuInfoFileOps;

impl CpuInfoFileOps {
    /// Creates a new inode for `/proc/cpuinfo`.
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self).parent(parent).build().unwrap()
    }
}

impl FileOps for CpuInfoFileOps {
    /// Retrieves the data for `/proc/cpuinfo`.
    fn data(&self) -> Result<Vec<u8>> {
        let output = all_cpus()
            .map(|cpu| CPU_INFORMATION.get_on_cpu(cpu).wait().to_string())
            .collect::<Vec<String>>()
            .join("\n");
        Ok(output.into_bytes())
    }
}

cpu_local! {
    static CPU_INFORMATION: Once<CpuInformation> = Once::new();
}

pub(super) fn init_on_each_cpu() {
    let guard = disable_preempt();
    CPU_INFORMATION
        .get_on_cpu(guard.current_cpu())
        .call_once(|| CpuInformation::new(&guard));
}
