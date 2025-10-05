// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{prelude::*, process::Gid};

pub fn sys_setgid(gid: i32, ctx: &Context) -> Result<SyscallReturn> {
    if gid < 0 {
        return_errno_with_message!(Errno::EINVAL, "GIDs cannot be negative");
    }

    let gid = Gid::new(gid.cast_unsigned());
    debug!("gid = {:?}", gid);

    let credentials = ctx.posix_thread.credentials_mut();
    credentials.set_gid(gid)?;

    Ok(SyscallReturn::Return(0))
}
