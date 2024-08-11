// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_getgid(ctx: &Context) -> Result<SyscallReturn> {
    let gid = ctx.posix_thread.credentials().rgid();

    Ok(SyscallReturn::Return(gid.as_u32() as _))
}
