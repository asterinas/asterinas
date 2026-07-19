// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_getppid(ctx: &Context) -> Result<SyscallReturn> {
    Ok(SyscallReturn::Return(ctx.process.parent().pid() as _))
}
