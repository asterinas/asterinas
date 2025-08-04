// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_umask(mask: u16, ctx: &Context) -> Result<SyscallReturn> {
    debug!("mask = 0o{:o}", mask);
    let old_mask = ctx.thread_local.borrow_fs().umask().write().set(mask);
    Ok(SyscallReturn::Return(old_mask as _))
}
