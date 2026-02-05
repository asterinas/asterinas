// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::file_table::{FdFlags, FileDesc, get_file_fast},
    prelude::*,
    process::{
        PidFile,
        posix_thread::{
            AsPosixThread,
            ptrace::{PtraceMode, check_may_access},
        },
    },
    syscall::SyscallReturn,
};

pub fn sys_pidfd_getfd(
    pidfd: FileDesc,
    targetfd: FileDesc,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    // The `flags` argument is reserved for future use. Currently, it must be specified as 0.
    if flags != 0 {
        return_errno_with_message!(Errno::EINVAL, "invalid flags");
    }
    debug!(
        "pidfd_getfd: pidfd={}, targetfd={}, flags={}",
        pidfd, targetfd, flags
    );

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, pidfd);
    let Some(pid_file) = file.downcast_ref::<PidFile>() else {
        return_errno_with_message!(Errno::EINVAL, "the file is not a PID file");
    };

    let process = pid_file
        .process_opt()
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "the target process has been reaped"))?;

    check_may_access(
        ctx.posix_thread,
        process.main_thread().as_posix_thread().unwrap(),
        PtraceMode::ATTACH_REALCREDS,
    )?;

    let main_thread = process.main_thread();

    // Get the file description corresponding to to the file descriptor `targetfd` in the process
    // referred to by the PID file.
    let target_file_table = main_thread.as_posix_thread().unwrap().file_table();
    let target_file = target_file_table
        .lock()
        .as_ref()
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "the target process has exited"))?
        .read()
        .get_file(targetfd)?
        .clone();

    // Duplicate the file descriptor into the caller's file descriptor table.
    let new_fd = {
        let mut file_table_locked = file_table.unwrap().write();
        file_table_locked.insert(target_file, FdFlags::CLOEXEC)
    };

    Ok(SyscallReturn::Return(new_fd as _))
}
