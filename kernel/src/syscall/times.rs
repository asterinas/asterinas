// SPDX-License-Identifier: MPL-2.0

use ostd::{mm::VmIo, timer::Jiffies};

use super::SyscallReturn;
use crate::{
    prelude::*,
    time::util::{clock_t, duration_to_clock_t, jiffies_to_clock_t},
};

pub fn sys_times(tms_addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    debug!("tms_addr = 0x{:x}", tms_addr);

    if tms_addr != 0 {
        let process = ctx.process.as_ref();
        let prof_clock = process.prof_clock();
        let (child_user_time, child_kernel_time) = process.reaped_children_stats().lock().get();
        let tms = Tms {
            tms_utime: jiffies_to_clock_t(prof_clock.user_clock().read_jiffies()),
            tms_stime: jiffies_to_clock_t(prof_clock.kernel_clock().read_jiffies()),
            tms_cutime: duration_to_clock_t(child_user_time),
            tms_cstime: duration_to_clock_t(child_kernel_time),
        };

        ctx.user_space().write_val(tms_addr, &tms)?;
    }

    Ok(SyscallReturn::Return(
        jiffies_to_clock_t(Jiffies::elapsed()) as isize,
    ))
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct Tms {
    tms_utime: clock_t,
    tms_stime: clock_t,
    tms_cutime: clock_t,
    tms_cstime: clock_t,
}
