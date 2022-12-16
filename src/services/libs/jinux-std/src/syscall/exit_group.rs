use crate::{log_syscall_entry, prelude::*};

use crate::syscall::{SyscallReturn, SYS_EXIT_GROUP};

/// Exit all thread in a process.
pub fn sys_exit_group(exit_code: u64) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_EXIT_GROUP);
    current!().exit(exit_code as _);
    Ok(SyscallReturn::Return(0))
}
