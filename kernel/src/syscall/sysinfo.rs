// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use aster_time::read_monotonic_time;
use aster_util::fixed_point::FixedU64;
use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{prelude::*, process::pid_table, sched::loadavg};

#[padding_struct]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
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
    type SysinfoLoadAvg = FixedU64<16>;

    let loadavg = loadavg::get_loadavg();
    let info = SysInfo {
        uptime: read_monotonic_time().as_secs_round_up() as i64,
        loads: [
            SysinfoLoadAvg::from(loadavg[0]).raw(),
            SysinfoLoadAvg::from(loadavg[1]).raw(),
            SysinfoLoadAvg::from(loadavg[2]).raw(),
        ],
        totalram: crate::vm::mem_total() as u64,
        freeram: osdk_frame_allocator::load_total_free_size() as u64,
        procs: pid_table::pid_table_mut().process_count() as u16,
        // `mem_unit` will always be 1 byte since Asterinas only supports
        // 64-bit CPU architectures.
        mem_unit: 1,
        ..Default::default() // TODO: add other system information
    };
    ctx.user_space().write_val(sysinfo_addr, &info)?;
    Ok(SyscallReturn::Return(0))
}

trait RoundUpSeconds {
    /// Returns the duration in seconds, rounding up a nonzero sub-second part.
    ///
    /// This matches the rounding used by Linux `sysinfo`.
    /// Reference: <https://elixir.bootlin.com/linux/v6.18/source/kernel/sys.c#L2904-L2906>
    fn as_secs_round_up(&self) -> u64;
}

impl RoundUpSeconds for Duration {
    fn as_secs_round_up(&self) -> u64 {
        self.as_secs()
            .saturating_add(u64::from(self.subsec_nanos() != 0))
    }
}

#[cfg(ktest)]
mod tests {
    use core::time::Duration;

    use ostd::prelude::ktest;

    use super::RoundUpSeconds;

    #[ktest]
    fn as_secs_round_up() {
        assert_eq!(Duration::new(0, 0).as_secs_round_up(), 0);
        assert_eq!(Duration::new(42, 0).as_secs_round_up(), 42);
        assert_eq!(Duration::new(42, 1).as_secs_round_up(), 43);
        assert_eq!(Duration::new(42, 999_999_999).as_secs_round_up(), 43);
    }
}
