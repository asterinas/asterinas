use crate::prelude::*;

use crate::{process::Process, syscall::SYS_EXIT};

use super::SyscallResult;

pub fn sys_exit(exit_code: i32) -> SyscallResult {
    debug!("[syscall][id={}][SYS_EXIT]", SYS_EXIT);
    Process::current().set_exit_code(exit_code);
    SyscallResult::Exit(exit_code)
}
