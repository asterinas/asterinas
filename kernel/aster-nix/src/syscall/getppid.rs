// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_getppid() -> Result<SyscallReturn> {
    let current = current!();
    let parent = current.parent();
    match parent {
        None => Ok(SyscallReturn::Return(0)),
        Some(parent) => Ok(SyscallReturn::Return(parent.pid() as _)),
    }
}
