use crate::{log_syscall_entry, prelude::*};

use crate::{process::Process, syscall::SYS_SCHED_YIELD};

use super::SyscallReturn;

pub fn sys_sched_yield() -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SCHED_YIELD);
    Process::yield_now();
    Ok(SyscallReturn::Return(0))
}
