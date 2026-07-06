// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{Gid, RawGid, posix_thread::ContextPthreadAdminApi},
};

pub fn sys_setregid(rgid: RawGid, egid: RawGid, ctx: &Context) -> Result<SyscallReturn> {
    let rgid = Gid::new(rgid);
    let egid = Gid::new(egid);

    debug!("rgid = {:?}, egid = {:?}", rgid, egid);

    let credentials = ctx.credentials_mut();
    credentials.set_regid(rgid, egid)?;

    Ok(SyscallReturn::Return(0))
}
