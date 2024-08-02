// SPDX-License-Identifier: MPL-2.0

use super::{CallingThreadInfo, SyscallReturn};
use crate::prelude::*;

pub fn sys_getppid(info: CallingThreadInfo) -> Result<SyscallReturn> {
    let current = info.pthread_info.process();
    let parent = current.parent();
    match parent {
        None => Ok(SyscallReturn::Return(0)),
        Some(parent) => Ok(SyscallReturn::Return(parent.pid() as _)),
    }
}
