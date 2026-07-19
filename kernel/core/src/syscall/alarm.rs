// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use super::SyscallReturn;
use crate::{
    prelude::*,
    time::{NSEC_PER_SEC, timer::Timeout},
};

pub fn sys_alarm(seconds: u32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("seconds = {}", seconds);

    let alarm_timer = ctx.process.timer_manager().alarm_timer();
    let mut timer_guard = alarm_timer.lock();

    let remaining = timer_guard.remain();
    let mut remaining_secs = remaining.as_secs();
    // Round up remaining time to match Linux kernel behavior.
    // Reference: <https://elixir.bootlin.com/linux/v6.15/source/kernel/time/itimer.c#L311-L319>
    if (remaining_secs == 0 && remaining.subsec_nanos() > 0)
        || remaining.subsec_nanos() as i64 >= NSEC_PER_SEC / 2
    {
        remaining_secs += 1;
    }

    if seconds == 0 {
        // Clear previous timer
        timer_guard.cancel();
        return Ok(SyscallReturn::Return(remaining_secs as _));
    }

    timer_guard.set_timeout(Timeout::After(Duration::from_secs(seconds as u64)));

    Ok(SyscallReturn::Return(remaining_secs as _))
}
