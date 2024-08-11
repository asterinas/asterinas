// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_getegid(ctx: &Context) -> Result<SyscallReturn> {
    let egid = ctx.posix_thread.credentials().egid();

    Ok(SyscallReturn::Return(egid.as_u32() as _))
}
