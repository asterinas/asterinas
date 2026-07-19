// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{prelude::*, thread::Thread};

pub fn sys_sched_yield(_ctx: &Context) -> Result<SyscallReturn> {
    Thread::yield_now();
    Ok(SyscallReturn::Return(0))
}
