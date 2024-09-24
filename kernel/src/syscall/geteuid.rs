// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{prelude::*, process::Uid};

pub fn sys_geteuid(ctx: &Context) -> Result<SyscallReturn> {
    let euid = ctx.posix_thread.credentials().euid();

    Ok(SyscallReturn::Return(<Uid as Into<u32>>::into(euid) as _))
}
