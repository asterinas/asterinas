// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{Gid, RawGid, posix_thread::ContextPthreadAdminApi},
};

pub fn sys_setresgid(
    rgid: RawGid,
    egid: RawGid,
    sgid: RawGid,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let rgid = Gid::new(rgid);
    let egid = Gid::new(egid);
    let sgid = Gid::new(sgid);

    debug!("rgid = {:?}, egid = {:?}, sgid = {:?}", rgid, egid, sgid);

    let credentials = ctx.credentials_mut();
    credentials.set_resgid(rgid, egid, sgid)?;

    Ok(SyscallReturn::Return(0))
}
