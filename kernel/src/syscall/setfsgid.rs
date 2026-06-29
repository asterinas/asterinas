// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{Gid, RawGid, posix_thread::ContextPthreadAdminApi},
};

pub fn sys_setfsgid(raw_gid: RawGid, ctx: &Context) -> Result<SyscallReturn> {
    let fsgid = Gid::new(raw_gid);

    debug!("fsgid = {:?}", fsgid);

    let old_fsgid = {
        let credentials = ctx.credentials_mut();
        credentials
            .set_fsgid(fsgid)
            .unwrap_or_else(|old_fsgid| old_fsgid)
    };

    Ok(SyscallReturn::Return(
        <Gid as Into<u32>>::into(old_fsgid) as _
    ))
}
