// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{prelude::*, process::Uid};

pub fn sys_setfsuid(uid: i32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("uid = {}", uid);

    let fsuid = if uid < 0 {
        None
    } else {
        Some(Uid::new(uid as u32))
    };

    let old_fsuid = {
        let credentials = ctx.posix_thread.credentials_mut();
        credentials.set_fsuid(fsuid)?
    };

    Ok(SyscallReturn::Return(
        <Uid as Into<u32>>::into(old_fsuid) as _
    ))
}
