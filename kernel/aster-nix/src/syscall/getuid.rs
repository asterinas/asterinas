// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_getuid(ctx: &Context) -> Result<SyscallReturn> {
    let uid = ctx.posix_thread.credentials().ruid();

    Ok(SyscallReturn::Return(uid.as_u32() as _))
}
