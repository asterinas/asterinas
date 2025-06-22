// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::FileDesc,
    prelude::*,
    time::{itimerspec_t, timerfd::TimerfdFile, timespec_t},
};

pub fn sys_timerfd_gettime(
    fd: FileDesc,
    itimerspec_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    if itimerspec_addr == 0 {
        return_errno_with_message!(Errno::EINVAL, "invalid pointer to return value");
    }

    let file_table = ctx.thread_local.borrow_file_table();
    let file_table_locked = file_table.unwrap().read();
    let timerfd_file = file_table_locked.get_file(fd as _)?;
    let timerfd_file = timerfd_file
        .downcast_ref::<TimerfdFile>()
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "the fd is not a timerfd"))?;

    let interval = timespec_t::from(timerfd_file.timer().interval());
    let remain = timespec_t::from(timerfd_file.timer().remain());
    let itimerspec = itimerspec_t {
        it_interval: interval,
        it_value: remain,
    };
    ctx.user_space().write_val(itimerspec_addr, &itimerspec)?;

    Ok(SyscallReturn::Return(0))
}
