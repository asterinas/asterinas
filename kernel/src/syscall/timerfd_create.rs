// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::FdFlags,
    prelude::*,
    time::{
        clockid_t,
        timerfd::{TFDFlags, TimerfdFile},
    },
};

pub fn sys_timerfd_create(clockid: clockid_t, flags: i32, ctx: &Context) -> Result<SyscallReturn> {
    let flags = TFDFlags::from_bits(flags as u32)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown flags"))?;

    let timerfd_file = TimerfdFile::new(clockid, flags, ctx)?;

    let fd = {
        let file_table = ctx.thread_local.borrow_file_table();
        let mut file_table_locked = file_table.unwrap().write();
        let fd_flags = if flags.contains(TFDFlags::TFD_CLOEXEC) {
            FdFlags::CLOEXEC
        } else {
            FdFlags::empty()
        };
        file_table_locked.insert(Arc::new(timerfd_file), fd_flags)
    };

    Ok(SyscallReturn::Return(fd as _))
}
