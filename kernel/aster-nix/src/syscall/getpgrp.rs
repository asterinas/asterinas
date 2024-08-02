// SPDX-License-Identifier: MPL-2.0

use super::{CallingThreadInfo, SyscallReturn};
use crate::prelude::*;

pub fn sys_getpgrp(info: CallingThreadInfo) -> Result<SyscallReturn> {
    let current = info.pthread_info.process();
    Ok(SyscallReturn::Return(current.pgid() as _))
}
