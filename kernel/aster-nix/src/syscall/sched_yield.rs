// SPDX-License-Identifier: MPL-2.0

use super::{CallingThreadInfo, SyscallReturn};
use crate::prelude::*;

pub fn sys_sched_yield(info: CallingThreadInfo) -> Result<SyscallReturn> {
    info.task_info_mut.yield_now();
    Ok(SyscallReturn::Return(0))
}
