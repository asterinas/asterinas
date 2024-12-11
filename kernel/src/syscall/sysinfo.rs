// SPDX-License-Identifier: MPL-2.0

use aster_time::read_monotonic_time;

use super::SyscallReturn;
use crate::prelude::*;

#[derive(Debug, Default, Clone, Copy, Pod)]
#[repr(C)]
pub struct sysinfo {
    uptime: i64,
    loads: [u64; 3],
    totalram: u64,
    freeram: u64,
    sharedram: u64,
    bufferram: u64,
    totalswap: u64,
    freeswap: u64,
    procs: u16,
    totalhigh: u64,
    freehigh: u64,
    mem_unit: u32,
}

pub fn sys_sysinfo(sysinfo_addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let info = sysinfo {
        uptime: read_monotonic_time().as_secs() as i64,
        ..Default::default() // TODO: add other system information
    };
    ctx.user_space().write_val(sysinfo_addr, &info)?;
    Ok(SyscallReturn::Return(0))
}
