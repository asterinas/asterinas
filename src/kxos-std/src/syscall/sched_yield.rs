use crate::prelude::*;

use crate::{process::Process, syscall::SYS_SCHED_YIELD};

use super::SyscallReturn;

pub fn sys_sched_yield() -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_SCHED_YIELD]", SYS_SCHED_YIELD);
    Process::yield_now();
    Ok(SyscallReturn::Return(0))
}
