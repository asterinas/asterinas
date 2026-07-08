// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use ostd::{
    mm::VmIo,
    timer::{Jiffies, TIMER_FREQ},
};

use super::SyscallReturn;
use crate::{prelude::*, time::NSEC_PER_SEC};

pub fn sys_times(tms_addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    debug!("tms_addr = 0x{:x}", tms_addr);

    if tms_addr != 0 {
        let process = ctx.process.as_ref();
        let prof_clock = process.prof_clock();
        let (child_user_time, child_kernel_time) = process.reaped_children_stats().lock().get();
        let tms = Tms {
            tms_utime: clock_t_from_jiffies(prof_clock.user_clock().read_jiffies().as_u64()),
            tms_stime: clock_t_from_jiffies(prof_clock.kernel_clock().read_jiffies().as_u64()),
            tms_cutime: clock_t_from_jiffies(duration_to_jiffies(child_user_time)),
            tms_cstime: clock_t_from_jiffies(duration_to_jiffies(child_kernel_time)),
        };

        ctx.user_space().write_val(tms_addr, &tms)?;
    }

    Ok(SyscallReturn::Return(
        clock_t_from_jiffies(Jiffies::elapsed().as_u64()) as isize,
    ))
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct Tms {
    tms_utime: i64,
    tms_stime: i64,
    tms_cutime: i64,
    tms_cstime: i64,
}

/// Converts a duration into kernel clock ticks.
fn duration_to_jiffies(duration: Duration) -> u64 {
    const NSEC_PER_JIFFY: u64 = NSEC_PER_SEC as u64 / TIMER_FREQ;
    const { assert!((NSEC_PER_SEC as u64).is_multiple_of(TIMER_FREQ)) };

    let sec_jiffies = duration.as_secs().saturating_mul(TIMER_FREQ);
    let subsec_jiffies = u64::from(duration.subsec_nanos()) / NSEC_PER_JIFFY;
    sec_jiffies.saturating_add(subsec_jiffies)
}

fn clock_t_from_jiffies(jiffies: u64) -> i64 {
    jiffies.min(i64::MAX as u64) as i64
}
