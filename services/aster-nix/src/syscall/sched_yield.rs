// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{log_syscall_entry, prelude::*, syscall::SYS_SCHED_YIELD, thread::Thread};

pub fn sys_sched_yield() -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SCHED_YIELD);
    Thread::yield_now();
    Ok(SyscallReturn::Return(0))
}
