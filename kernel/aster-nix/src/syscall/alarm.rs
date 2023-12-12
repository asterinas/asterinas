// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use super::{SyscallReturn, SYS_ALARM};
use crate::{log_syscall_entry, prelude::*, process::posix_thread::PosixThreadExt};

pub fn sys_alarm(seconds: u32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_ALARM);
    debug!("seconds = {}", seconds);

    let current_thread = current_thread!();
    let mut real_timer = {
        let posix_thread = current_thread.as_posix_thread().unwrap();
        posix_thread.real_timer().lock()
    };

    let remaining_secs = real_timer.remain().as_secs();

    if seconds == 0 {
        // Clear previous timer
        real_timer.clear();
        return Ok(SyscallReturn::Return(remaining_secs as _));
    }

    real_timer.set(Duration::from_secs(seconds as u64))?;

    Ok(SyscallReturn::Return(remaining_secs as _))
}
