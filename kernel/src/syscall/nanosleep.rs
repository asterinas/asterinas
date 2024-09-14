// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use ostd::sync::Waiter;

use super::{clock_gettime::read_clock, ClockId, SyscallReturn};
use crate::{
    prelude::*,
    time::{clockid_t, timespec_t, TIMER_ABSTIME},
};

pub fn sys_nanosleep(
    request_timespec_addr: Vaddr,
    remain_timespec_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let clockid = ClockId::CLOCK_MONOTONIC;

    do_clock_nanosleep(
        clockid as clockid_t,
        false,
        request_timespec_addr,
        remain_timespec_addr,
        ctx,
    )
}

pub fn sys_clock_nanosleep(
    clockid: clockid_t,
    flags: i32,
    request_timespec_addr: Vaddr,
    remain_timespec_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let is_abs_time = if flags == 0 {
        false
    } else if flags == TIMER_ABSTIME {
        true
    } else {
        unreachable!()
    };

    do_clock_nanosleep(
        clockid,
        is_abs_time,
        request_timespec_addr,
        remain_timespec_addr,
        ctx,
    )
}

fn do_clock_nanosleep(
    clockid: clockid_t,
    is_abs_time: bool,
    request_timespec_addr: Vaddr,
    remain_timespec_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let request_time = {
        let timespec = ctx
            .get_user_space()
            .read_val::<timespec_t>(request_timespec_addr)?;
        Duration::try_from(timespec)?
    };

    debug!(
        "clockid = {:?}, is_abs_time = {}, request_time = {:?}, remain_timespec_addr = 0x{:x}",
        clockid, is_abs_time, request_time, remain_timespec_addr
    );

    let start_time = read_clock(clockid, ctx)?;
    let timeout = if is_abs_time {
        if request_time < start_time {
            return Ok(SyscallReturn::Return(0));
        }

        request_time - start_time
    } else {
        request_time
    };

    // FIXME: sleeping thread can only be interrupted by signals that will call signal handler or terminate
    // current process. i.e., the signals that should be ignored will not interrupt sleeping thread.
    let waiter = Waiter::new_pair().0;

    let res = waiter.pause_until_or_timeout(|| None, &timeout);
    match res {
        Err(e) if e.error() == Errno::ETIME => Ok(SyscallReturn::Return(0)),
        Err(e) if e.error() == Errno::EINTR => {
            let end_time = read_clock(clockid, ctx)?;

            if end_time >= start_time + timeout {
                return Ok(SyscallReturn::Return(0));
            }

            if remain_timespec_addr != 0 && !is_abs_time {
                let remaining_duration = (start_time + timeout) - end_time;
                let remaining_timespec = timespec_t::from(remaining_duration);
                ctx.get_user_space()
                    .write_val(remain_timespec_addr, &remaining_timespec)?;
            }

            return_errno_with_message!(Errno::EINTR, "sleep was interrupted");
        }
        Ok(()) | Err(_) => unreachable!(),
    }
}
