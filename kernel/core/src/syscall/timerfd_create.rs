// SPDX-License-Identifier: MPL-2.0

use super::{ClockId, SyscallReturn};
use crate::{
    fs::file::file_table::FdFlags,
    prelude::*,
    time::{
        clockid_t,
        timerfd::{TFDFlags, TimerfdFile},
    },
};

pub(super) fn sys_timerfd_create(
    clockid: clockid_t,
    flags: i32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let flags = TFDFlags::from_bits(flags as u32)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown flags"))?;

    let clock_id = ClockId::try_from(clockid)
        .map_err(|_| Error::with_message(Errno::EINVAL, "invalid clock ID"))?;

    let timerfd_file = TimerfdFile::new(clock_id, flags, ctx)?;

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

    Ok(SyscallReturn::Return(fd.into()))
}
