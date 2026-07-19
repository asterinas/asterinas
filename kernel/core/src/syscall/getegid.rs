// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{prelude::*, process::Gid};

pub fn sys_getegid(ctx: &Context) -> Result<SyscallReturn> {
    let egid = ctx.posix_thread.credentials().egid();

    Ok(SyscallReturn::Return(<Gid as Into<u32>>::into(egid) as _))
}
