// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use super::SyscallReturn;
use crate::{
    prelude::*,
    time::{itimerval_t, timer::Timeout, timeval_t},
};

/// `ItimerType` is used to differ the target timer for some timer-related syscalls.
#[derive(Debug, Copy, Clone, TryFromInt, PartialEq)]
#[repr(i32)]
#[allow(non_camel_case_types)]
pub(super) enum ItimerType {
    ITIMER_REAL = 0,
    ITIMER_VIRTUAL = 1,
    ITIMER_PROF = 2,
}

pub fn sys_setitimer(
    itimer_type: i32,
    new_itimerval_addr: Vaddr,
    old_itimerval_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "itimer_type = {}, new_itimerval_addr = 0x{:x}, old_itimerval_addr = 0x{:x}, ",
        itimer_type, new_itimerval_addr, old_itimerval_addr
    );

    if new_itimerval_addr == 0 {
        return_errno_with_message!(Errno::EINVAL, "invalid pointer to new value");
    }
    let user_space = ctx.user_space();
    let new_itimerval = user_space.read_val::<itimerval_t>(new_itimerval_addr)?;
    let interval = Duration::try_from(new_itimerval.it_interval)?;
    let expire_time = Duration::try_from(new_itimerval.it_value)?;

    let process_timer_manager = ctx.process.timer_manager();
    let timer = match ItimerType::try_from(itimer_type)? {
        ItimerType::ITIMER_REAL => process_timer_manager.alarm_timer(),
        ItimerType::ITIMER_VIRTUAL => process_timer_manager.virtual_timer(),
        ItimerType::ITIMER_PROF => process_timer_manager.prof_timer(),
    };

    if old_itimerval_addr > 0 {
        let old_interval = timeval_t::from(timer.interval());
        let remain = timeval_t::from(timer.remain());
        let old_itimerval = itimerval_t {
            it_interval: old_interval,
            it_value: remain,
        };
        user_space.write_val(old_itimerval_addr, &old_itimerval)?;
    }

    timer.set_interval(interval);
    if expire_time == Duration::ZERO {
        // Clear previous timer
        timer.cancel();
    } else {
        timer.set_timeout(Timeout::After(expire_time));
    }

    Ok(SyscallReturn::Return(0))
}

pub fn sys_getitimer(
    itimer_type: i32,
    itimerval_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "itimer_type = {}, itimerval_addr = 0x{:x}",
        itimer_type, itimerval_addr
    );

    if itimerval_addr == 0 {
        return_errno_with_message!(Errno::EINVAL, "invalid pointer to itimerval");
    }

    let process_timer_manager = ctx.process.timer_manager();
    let timer = match ItimerType::try_from(itimer_type)? {
        ItimerType::ITIMER_REAL => process_timer_manager.alarm_timer(),
        ItimerType::ITIMER_VIRTUAL => process_timer_manager.virtual_timer(),
        ItimerType::ITIMER_PROF => process_timer_manager.prof_timer(),
    };

    let interval = timeval_t::from(timer.interval());
    let remain = timeval_t::from(timer.remain());
    let itimerval = itimerval_t {
        it_interval: interval,
        it_value: remain,
    };
    ctx.user_space().write_val(itimerval_addr, &itimerval)?;

    Ok(SyscallReturn::Return(0))
}
