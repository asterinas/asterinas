// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{prelude::*, process::Uid};

pub fn sys_setresuid(ruid: i32, euid: i32, suid: i32, ctx: &Context) -> Result<SyscallReturn> {
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

    let suid = if suid > 0 {
        Some(Uid::new(suid as u32))
    } else {
        None
    };

    debug!("ruid = {:?}, euid = {:?}, suid = {:?}", ruid, euid, suid);

    let credentials = ctx.posix_thread.credentials_mut();

    credentials.set_resuid(ruid, euid, suid)?;

    Ok(SyscallReturn::Return(0))
}
