// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_getpgrp(ctx: &Context) -> Result<SyscallReturn> {
    Ok(SyscallReturn::Return(ctx.process.pgid() as _))
}
