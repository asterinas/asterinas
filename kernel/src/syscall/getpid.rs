// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_getpid(ctx: &Context) -> Result<SyscallReturn> {
    let pid = ctx.process.pid();
    debug!("[sys_getpid]: pid = {}", pid);
    Ok(SyscallReturn::Return(pid as _))
}
