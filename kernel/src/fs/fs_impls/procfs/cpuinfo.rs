// SPDX-License-Identifier: MPL-2.0

//! This module offers `/proc/cpuinfo` file support, which provides
//! information about the CPU architecture, cores, and other details.
//!
//! Reference: <https://man7.org/linux/man-pages/man5/proc_cpuinfo.5.html>

use aster_util::printer::VmPrinter;
use ostd::{
    cpu::{PinCurrentCpu, all_cpus},
    cpu_local,
    task::disable_preempt,
};
use spin::Once;

use crate::{
    arch::cpu::CpuInformation,
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{Inode, mkmod},
    },
    prelude::*,
};

/// Represents the inode at `/proc/cpuinfo`.
pub struct CpuInfoFileOps;

impl CpuInfoFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference:
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/cpuinfo.c#L25>
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/generic.c#L549-L550>
        ProcFileBuilder::new(Self, mkmod!(a+r))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for CpuInfoFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        for cpu in all_cpus() {
            let cpu_info = CPU_INFORMATION.get_on_cpu(cpu).get().unwrap().to_string();
            writeln!(printer, "{}", cpu_info)?;
        }

        Ok(printer.bytes_written())
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
