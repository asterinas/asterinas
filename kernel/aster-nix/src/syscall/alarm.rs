// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_alarm(seconds: u32) -> Result<SyscallReturn> {
    debug!("seconds = {}", seconds);

    let current = current!();
    let alarm_timer = current.alarm_timer();

    let remaining = alarm_timer.remain();
    let mut remaining_secs = remaining.as_secs();
    if remaining.subsec_nanos() > 0 {
        remaining_secs += 1;
    }

    if seconds == 0 {
        // Clear previous timer
        alarm_timer.cancel();
        return Ok(SyscallReturn::Return(remaining_secs as _));
    }

    alarm_timer.set_timeout(Duration::from_secs(seconds as u64));

    Ok(SyscallReturn::Return(remaining_secs as _))
}
