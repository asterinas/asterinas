// SPDX-License-Identifier: MPL-2.0

use crate::{
    prelude::*,
    process::{posix_thread::PosixThreadExt, TermStatus},
    syscall::SyscallReturn,
};

pub fn sys_exit(exit_code: i32) -> Result<SyscallReturn> {
    debug!("exid code = {}", exit_code);

    let current_thread = current_thread!();
    current_thread.exit();

    let tid = current_thread.tid();
    let pid = current!().pid();
    debug!("tid = {}, pid = {}", tid, pid);

    let posix_thread = current_thread.as_posix_thread().unwrap();
    posix_thread.exit(tid, TermStatus::Exited(exit_code as _))?;

    Ok(SyscallReturn::Return(0))
}
