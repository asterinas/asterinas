// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_getpgrp(_ctx: &Context) -> Result<SyscallReturn> {
    let current = current!();
    Ok(SyscallReturn::Return(current.pgid() as _))
}
