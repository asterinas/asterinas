// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{prelude::*, process::Gid};

pub fn sys_setregid(rgid: i32, egid: i32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("rgid = {}, egid = {}", rgid, egid);

    let rgid = if rgid > 0 {
        Some(Gid::new(rgid as u32))
    } else {
        None
    };

    let egid = if egid > 0 {
        Some(Gid::new(egid as u32))
    } else {
        None
    };

    let credentials = ctx.posix_thread.credentials_mut();
    credentials.set_regid(rgid, egid)?;

    Ok(SyscallReturn::Return(0))
}
