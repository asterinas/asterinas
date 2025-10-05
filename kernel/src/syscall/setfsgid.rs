// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{prelude::*, process::Gid};

pub fn sys_setfsgid(gid: i32, ctx: &Context) -> Result<SyscallReturn> {
    let fsgid = if gid >= 0 {
        Some(Gid::new(gid.cast_unsigned()))
    } else {
        None
    };

    debug!("fsgid = {:?}", fsgid);

    let old_fsgid = {
        let credentials = ctx.posix_thread.credentials_mut();
        credentials
            .set_fsgid(fsgid)
            .unwrap_or_else(|old_fsgid| old_fsgid)
    };

    Ok(SyscallReturn::Return(
        <Gid as Into<u32>>::into(old_fsgid) as _
    ))
}
