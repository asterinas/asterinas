// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{prelude::*, process::Gid};

pub fn sys_setfsgid(gid: i32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("gid = {}", gid);

    let fsgid = if gid < 0 {
        None
    } else {
        Some(Gid::new(gid as u32))
    };

    let old_fsgid = {
        let credentials = ctx.posix_thread.credentials_mut();
        credentials.set_fsgid(fsgid)?
    };

    Ok(SyscallReturn::Return(
        <Gid as Into<u32>>::into(old_fsgid) as _
    ))
}
