// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_gettid(ctx: &Context) -> Result<SyscallReturn> {
    let tid = ctx.posix_thread.tid();
    Ok(SyscallReturn::Return(tid as _))
}
