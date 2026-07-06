// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{RawUid, Uid, posix_thread::ContextPthreadAdminApi},
};

pub fn sys_setuid(raw_uid: RawUid, ctx: &Context) -> Result<SyscallReturn> {
    let uid = Uid::try_from(raw_uid)?;
    debug!("uid = {:?}", uid);

    let credentials = ctx.credentials_mut();
    credentials.set_uid(uid)?;

    Ok(SyscallReturn::Return(0))
}
