// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_geteuid(ctx: &Context) -> Result<SyscallReturn> {
    let euid = ctx.posix_thread.credentials().euid();

    Ok(SyscallReturn::Return(euid.as_u32() as _))
}
