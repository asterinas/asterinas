// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use super::SyscallReturn;
use crate::{
    prelude::*,
    time::{itimerspec_t, timer::Timeout, timespec_t, TIMER_ABSTIME},
};

pub fn sys_timer_settime(
    timer_id: usize,
    flags: i32,
    new_itimerspec_addr: Vaddr,
    old_itimerspec_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    if new_itimerspec_addr == 0 {
        return_errno_with_message!(Errno::EINVAL, "invalid pointer to new value");
    }

    let user_space = ctx.user_space();
    let new_itimerspec = user_space.read_val::<itimerspec_t>(new_itimerspec_addr)?;
    let interval = Duration::try_from(new_itimerspec.it_interval)?;
    let expire_time = Duration::try_from(new_itimerspec.it_value)?;

    let Some(timer) = ctx.process.timer_manager().find_posix_timer(timer_id) else {
        return_errno_with_message!(Errno::EINVAL, "invalid timer ID");
    };

    if old_itimerspec_addr > 0 {
        let old_interval = timespec_t::from(timer.interval());
        let remain = timespec_t::from(timer.remain());
        let old_itimerspec = itimerspec_t {
            it_interval: old_interval,
            it_value: remain,
        };
        user_space.write_val(old_itimerspec_addr, &old_itimerspec)?;
    }

    timer.set_interval(interval);
    if expire_time == Duration::ZERO {
        // Clear previous timer
        timer.cancel();
    } else {
        let timeout = if flags == 0 {
            Timeout::After(expire_time)
        } else if flags == TIMER_ABSTIME {
            Timeout::When(expire_time)
        } else {
            unreachable!()
        };
        timer.set_timeout(timeout);
    }

    Ok(SyscallReturn::Return(0))
}

pub fn sys_timer_gettime(
    timer_id: usize,
    itimerspec_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    if itimerspec_addr == 0 {
        return_errno_with_message!(Errno::EINVAL, "invalid pointer to return value");
    }
    let Some(timer) = ctx.process.timer_manager().find_posix_timer(timer_id) else {
        return_errno_with_message!(Errno::EINVAL, "invalid timer ID");
    };

    let interval = timespec_t::from(timer.interval());
    let remain = timespec_t::from(timer.remain());
    let itimerspec = itimerspec_t {
        it_interval: interval,
        it_value: remain,
    };
    ctx.user_space().write_val(itimerspec_addr, &itimerspec)?;

    Ok(SyscallReturn::Return(0))
}
