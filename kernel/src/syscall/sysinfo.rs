// SPDX-License-Identifier: MPL-2.0

use aster_time::read_monotonic_time;

use super::SyscallReturn;
use crate::prelude::*;

#[derive(Debug, Default, Clone, Copy, Pod)]
#[repr(C)]
pub struct sysinfo {
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
    let info = sysinfo {
        uptime: read_monotonic_time().as_secs() as i64,
        totalram: crate::vm::mem_total() as u64,
        freeram: osdk_frame_allocator::load_total_free_size() as u64,
        ..Default::default() // TODO: add other system information
    };
    ctx.user_space().write_val(sysinfo_addr, &info)?;
    Ok(SyscallReturn::Return(0))
}
