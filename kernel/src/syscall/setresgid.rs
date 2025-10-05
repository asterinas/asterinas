// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{prelude::*, process::Gid};

pub fn sys_setresgid(rgid: i32, egid: i32, sgid: i32, ctx: &Context) -> Result<SyscallReturn> {
    let rgid = if rgid >= 0 {
        Some(Gid::new(rgid.cast_unsigned()))
    } else {
        None
    };

    let egid = if egid >= 0 {
        Some(Gid::new(egid.cast_unsigned()))
    } else {
        None
    };

    let sgid = if sgid >= 0 {
        Some(Gid::new(sgid.cast_unsigned()))
    } else {
        None
    };

    debug!("rgid = {:?}, egid = {:?}, sgid = {:?}", rgid, egid, sgid);

    let credentials = ctx.posix_thread.credentials_mut();
    credentials.set_resgid(rgid, egid, sgid)?;

    Ok(SyscallReturn::Return(0))
}
