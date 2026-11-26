// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use ostd::{mm::VmIo, sync::Waiter};

use super::{ClockId, SyscallReturn};
use crate::{
    prelude::*,
    time::{
        TIMER_ABSTIME, clockid_t,
        clocks::{BootTimeClock, MonotonicClock, RealTimeClock},
        timer::Timeout,
        timespec_t,
        wait::ManagedTimeout,
    },
};

pub(super) fn sys_nanosleep(
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

pub(super) fn sys_clock_nanosleep(
    clockid: clockid_t,
    flags: i32,
    request_timespec_addr: Vaddr,
    remain_timespec_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let is_abs_time = (flags & TIMER_ABSTIME) != 0;

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
            .user_space()
            .read_val::<timespec_t>(request_timespec_addr)?;
        Duration::try_from(timespec)?
    };

    debug!(
        "clockid = {:?}, is_abs_time = {}, request_time = {:?}, remain_timespec_addr = 0x{:x}",
        clockid, is_abs_time, request_time, remain_timespec_addr
    );

    let timer_manager = {
        let clock_id = ClockId::try_from(clockid)?;
        match clock_id {
            ClockId::CLOCK_BOOTTIME => BootTimeClock::timer_manager(),
            ClockId::CLOCK_MONOTONIC => MonotonicClock::timer_manager(),
            ClockId::CLOCK_REALTIME => RealTimeClock::timer_manager(),
            // FIXME: We should better not expose this profiling timer manager.
            ClockId::CLOCK_PROCESS_CPUTIME_ID => {
                ctx.process.timer_manager().prof_timer().timer_manager()
            }
            // Linux returns `EOPNOTSUPP` for `CLOCK_THREAD_CPUTIME_ID`,
            // `CLOCK_MONOTONIC_RAW`, and the low-resolution coarse clocks.
            ClockId::CLOCK_THREAD_CPUTIME_ID
            | ClockId::CLOCK_MONOTONIC_RAW
            | ClockId::CLOCK_REALTIME_COARSE
            | ClockId::CLOCK_MONOTONIC_COARSE => {
                return Err(errno!(EOPNOTSUPP, "unsupported clock ID"));
            }
        }
    };

    let timeout = if is_abs_time {
        Timeout::When(request_time)
    } else {
        Timeout::After(request_time)
    };
    let mut managed_timeout = ManagedTimeout::new_with_manager(timeout, timer_manager);
    managed_timeout.freeze();

    let waiter = Waiter::new_pair().0;
    let res = waiter.pause_until_or_timeout(|| None::<!>, managed_timeout.clone());

    match res {
        Err(e) if e.error() == Errno::ETIME => Ok(SyscallReturn::Return(0)),
        Err(e) if e.error() == Errno::ERESTARTSYS => {
            let remaining_duration = managed_timeout.remain();
            if remaining_duration.is_zero() {
                return Ok(SyscallReturn::Return(0));
            }

            if remain_timespec_addr != 0 && !is_abs_time {
                let remaining_timespec = timespec_t::from(remaining_duration);
                ctx.user_space()
                    .write_val(remain_timespec_addr, &remaining_timespec)?;
            }

            Err(errno!(EINTR, "sleep was interrupted"))
        }
        Err(_) => unreachable!(),
    }
}
