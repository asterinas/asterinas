use crate::process::posix_thread::posix_thread_ext::PosixThreadExt;
use crate::{log_syscall_entry, prelude::*};

use crate::syscall::{SyscallReturn, SYS_EXIT};

pub fn sys_exit(exit_code: i32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_EXIT);
    debug!("exid code = {}", exit_code);
    let current_thread = current_thread!();
    let tid = current_thread.tid();
    let current = current!();
    let pid = current.pid();
    debug!("tid = {}, pid = {}", tid, pid);
    let posix_thread = current_thread.as_posix_thread().unwrap();
    current_thread.exit();
    posix_thread.exit(tid, exit_code)?;

    Ok(SyscallReturn::Return(0))
}
