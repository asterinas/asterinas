// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{RawUid, Uid, posix_thread::ContextPthreadAdminApi},
};

pub fn sys_setfsuid(raw_uid: RawUid, ctx: &Context) -> Result<SyscallReturn> {
    let fsuid = Uid::new(raw_uid);

    debug!("fsuid = {:?}", fsuid);

    let old_fsuid = {
        let credentials = ctx.credentials_mut();
        credentials
            .set_fsuid(fsuid)
            .unwrap_or_else(|old_fsuid| old_fsuid)
    };

    Ok(SyscallReturn::Return(
        <Uid as Into<u32>>::into(old_fsuid) as _
    ))
}
