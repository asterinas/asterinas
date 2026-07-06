// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{RawUid, Uid, posix_thread::ContextPthreadAdminApi},
};

pub fn sys_setreuid(ruid: RawUid, euid: RawUid, ctx: &Context) -> Result<SyscallReturn> {
    let ruid = Uid::new(ruid);
    let euid = Uid::new(euid);

    debug!("ruid = {:?}, euid = {:?}", ruid, euid);

    let credentials = ctx.credentials_mut();
    credentials.set_reuid(ruid, euid)?;

    Ok(SyscallReturn::Return(0))
}
