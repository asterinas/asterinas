// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use super::SYS_CLOCK_GETTIME;
use crate::time::now_as_duration;
use crate::{
    log_syscall_entry,
    prelude::*,
    time::{clockid_t, timespec_t, ClockID},
    util::write_val_to_user,
};

pub fn sys_clock_gettime(clockid: clockid_t, timespec_addr: Vaddr) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_CLOCK_GETTIME);
    let clock_id = ClockID::try_from(clockid)?;
    debug!("clockid = {:?}", clock_id);

    let time_duration = now_as_duration(&clock_id)?;

    let timespec = timespec_t::from(time_duration);
    write_val_to_user(timespec_addr, &timespec)?;

    Ok(SyscallReturn::Return(0))
}
