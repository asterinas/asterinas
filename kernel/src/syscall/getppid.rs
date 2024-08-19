// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_getppid(ctx: &Context) -> Result<SyscallReturn> {
    let parent = ctx.process.parent();
    match parent {
        None => Ok(SyscallReturn::Return(0)),
        Some(parent) => Ok(SyscallReturn::Return(parent.pid() as _)),
    }
}
