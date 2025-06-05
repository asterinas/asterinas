// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{file_table::FdFlags, utils::StatusFlags},
    prelude::*,
    process::{process_table, Pid, PidFile},
    syscall::SyscallReturn,
};

pub fn sys_pidfd_open(pid: Pid, flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    let is_nonblocking = {
        let flags = PidfdFlags::from_bits(flags)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid flags"))?;
        debug!("pid = {}, flags = {:?}", pid, flags);
        flags.contains(PidfdFlags::PIDFD_NONBLOCK)
    };

    if pid.cast_signed() < 0 {
        return_errno_with_message!(Errno::EINVAL, "all negative PIDs are not valid");
    }

    let process = process_table::get_process(pid)
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "the process does not exist"))?;

    let pid_fd = {
        let pid_file = Arc::new(PidFile::new(process, is_nonblocking));
        let file_table = ctx.thread_local.borrow_file_table();
        let mut file_table_locked = file_table.unwrap().write();
        // "the close-on-exec flag is set on the file descriptor."
        // Reference: <https://man7.org/linux/man-pages/man2/pidfd_open.2.html>.
        file_table_locked.insert(pid_file, FdFlags::CLOEXEC)
    };

    Ok(SyscallReturn::Return(pid_fd as _))
}

bitflags! {
    struct PidfdFlags: u32 {
        const PIDFD_NONBLOCK = StatusFlags::O_NONBLOCK.bits();
    }
}
