use crate::{log_syscall_entry, prelude::*};

use crate::syscall::{SyscallReturn, SYS_EXIT};

pub fn sys_exit(exit_code: i32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_EXIT);
    current!().exit(exit_code);
    Ok(SyscallReturn::Return(0))
}
