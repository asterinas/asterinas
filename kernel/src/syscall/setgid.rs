// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{Gid, RawGid, posix_thread::ContextPthreadAdminApi},
};

pub fn sys_setgid(raw_gid: RawGid, ctx: &Context) -> Result<SyscallReturn> {
    let gid = Gid::try_from(raw_gid)?;
    debug!("gid = {:?}", gid);

    let credentials = ctx.credentials_mut();
    credentials.set_gid(gid)?;

    Ok(SyscallReturn::Return(0))
}
