use kxos_frame::debug;

use crate::{process::Process, syscall::SYS_SCHED_YIELD};

use super::SyscallResult;

pub fn sys_sched_yield() -> SyscallResult {
    debug!("[syscall][id={}][SYS_SCHED_YIELD]", SYS_SCHED_YIELD);
    Process::yield_now();
    SyscallResult::Return(0)
}
