use crate::prelude::*;

use crate::syscall::{SyscallReturn, SYS_EXIT};

pub fn sys_exit(exit_code: i32) -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_EXIT]", SYS_EXIT);
    current!().exit(exit_code);
    Ok(SyscallReturn::Return(0))
}
