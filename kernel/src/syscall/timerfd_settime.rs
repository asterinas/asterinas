// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    fs::file::file_table::FileDesc,
    prelude::*,
    time::{
        itimerspec_t,
        timerfd::{TFDSetTimeFlags, TimerfdFile},
        timespec_t,
    },
};

pub fn sys_timerfd_settime(
    fd: FileDesc,
    flags: i32,
    new_itimerspec_addr: Vaddr,
    old_itimerspec_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let flags = TFDSetTimeFlags::from_bits(flags as u32)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid flags for timerfd_settime"))?;
    let file_table = ctx.thread_local.borrow_file_table();
    let file_table_locked = file_table.unwrap().read();
    let timerfd_file = file_table_locked.get_file(fd as _)?;
    let timerfd_file = timerfd_file
        .downcast_ref::<TimerfdFile>()
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "the fd is not a timerfd"))?;

    let user_space = ctx.user_space();
    let new_itimerspec = user_space.read_val::<itimerspec_t>(new_itimerspec_addr)?;
    let interval = Duration::try_from(new_itimerspec.it_interval)?;
    let expire_time = Duration::try_from(new_itimerspec.it_value)?;

    let (old_interval, remain) = timerfd_file.set_time(expire_time, interval, flags);
    if old_itimerspec_addr > 0 {
        let old_interval = timespec_t::from(old_interval);
        let remain = timespec_t::from(remain);
        let old_itimerspec = itimerspec_t {
            it_interval: old_interval,
            it_value: remain,
        };
        user_space.write_val(old_itimerspec_addr, &old_itimerspec)?;
    }

    Ok(SyscallReturn::Return(0))
}
