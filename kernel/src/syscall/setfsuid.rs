// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{prelude::*, process::Uid};

pub fn sys_setfsuid(uid: i32, ctx: &Context) -> Result<SyscallReturn> {
    let fsuid = if uid >= 0 {
        Some(Uid::new(uid.cast_unsigned()))
    } else {
        None
    };

    debug!("fsuid = {:?}", fsuid);

    let old_fsuid = {
        let credentials = ctx.posix_thread.credentials_mut();
        credentials
            .set_fsuid(fsuid)
            .unwrap_or_else(|old_fsuid| old_fsuid)
    };

    Ok(SyscallReturn::Return(
        <Uid as Into<u32>>::into(old_fsuid) as _
    ))
}
