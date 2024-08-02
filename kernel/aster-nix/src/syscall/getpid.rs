// SPDX-License-Identifier: MPL-2.0

use super::{CallingThreadInfo, SyscallReturn};
use crate::prelude::*;

pub fn sys_getpid(info: CallingThreadInfo) -> Result<SyscallReturn> {
    let pid = info.pthread_info.process().pid();
    debug!("[sys_getpid]: pid = {}", pid);
    Ok(SyscallReturn::Return(pid as _))
}
