// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use super::{SyscallReturn, SYS_ALARM};
use crate::{log_syscall_entry, prelude::*};

pub fn sys_alarm(seconds: u32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_ALARM);
    debug!("seconds = {}", seconds);

    let current = current!();
    let real_timer = current.real_timer();

    let remaining_secs = real_timer.remain().as_secs();

    if seconds == 0 {
        // Clear previous timer
        real_timer.clear();
        return Ok(SyscallReturn::Return(remaining_secs as _));
    }

    real_timer.set_timer(Duration::from_secs(seconds as u64));

    Ok(SyscallReturn::Return(remaining_secs as _))
}
