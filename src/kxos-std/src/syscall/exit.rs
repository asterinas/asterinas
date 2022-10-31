use crate::prelude::*;

use crate::syscall::SYS_EXIT;

use super::SyscallResult;

pub fn sys_exit(exit_code: i32) -> SyscallResult {
    debug!("[syscall][id={}][SYS_EXIT]", SYS_EXIT);
    current!().exit(exit_code);
    SyscallResult::NotReturn
}
