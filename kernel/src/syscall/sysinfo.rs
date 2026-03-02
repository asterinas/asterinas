// SPDX-License-Identifier: MPL-2.0

use aster_time::read_monotonic_time;
use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{prelude::*, process::process_table};

#[repr(C)]
#[padding_struct]
#[derive(Debug, Default, Clone, Copy, Pod)]
struct SysInfo {
    uptime: i64,     /* Seconds since boot */
    loads: [u64; 3], /* 1, 5, and 15 minute load averages */
    totalram: u64,   /* Total usable main memory size */
    freeram: u64,    /* Available memory size */
    sharedram: u64,  /* Amount of shared memory */
    bufferram: u64,  /* Memory used by buffers */
    totalswap: u64,  /* Total swap space size */
    freeswap: u64,   /* swap space still available */
    procs: u16,      /* Number of current processes */
    totalhigh: u64,  /* Total high memory size */
    freehigh: u64,   /* Available high memory size */
    mem_unit: u32,   /* Memory unit size in bytes */
}

pub fn sys_sysinfo(sysinfo_addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let info = SysInfo {
        uptime: read_monotonic_time().as_secs() as i64,
        totalram: crate::vm::mem_total() as u64,
        freeram: osdk_frame_allocator::load_total_free_size() as u64,
        procs: process_table::process_num() as u16,
        // `mem_unit` will always be 1 byte since Asterinas only supports
        // 64-bit CPU architectures.
        mem_unit: 1,
        ..Default::default() // TODO: add other system information
    };
    ctx.user_space().write_val(sysinfo_addr, &info)?;
    Ok(SyscallReturn::Return(0))
}
