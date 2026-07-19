// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{prelude::*, process::Uid};

pub fn sys_getuid(ctx: &Context) -> Result<SyscallReturn> {
    let uid = ctx.posix_thread.credentials().ruid();

    Ok(SyscallReturn::Return(<Uid as Into<u32>>::into(uid) as _))
}
