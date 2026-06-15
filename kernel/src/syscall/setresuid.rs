// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{RawUid, Uid, posix_thread::ContextPthreadAdminApi},
};

pub fn sys_setresuid(
    ruid: RawUid,
    euid: RawUid,
    suid: RawUid,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let ruid = Uid::new(ruid);
    let euid = Uid::new(euid);
    let suid = Uid::new(suid);

    debug!("ruid = {:?}, euid = {:?}, suid = {:?}", ruid, euid, suid);

    let credentials = ctx.credentials_mut();
    credentials.set_resuid(ruid, euid, suid)?;

    Ok(SyscallReturn::Return(0))
}
