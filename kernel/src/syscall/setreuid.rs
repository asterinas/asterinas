// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{prelude::*, process::Uid};

pub fn sys_setreuid(ruid: i32, euid: i32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("ruid = {}, euid = {}", ruid, euid);

    let ruid = if ruid > 0 {
        Some(Uid::new(ruid as u32))
    } else {
        None
    };

    let euid = if euid > 0 {
        Some(Uid::new(euid as u32))
    } else {
        None
    };

    let credentials = ctx.posix_thread.credentials_mut();
    credentials.set_reuid(ruid, euid)?;

    Ok(SyscallReturn::Return(0))
}
