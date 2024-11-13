// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{prelude::*, time::SystemTime};

pub fn sys_time(tloc: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    debug!("tloc = 0x{tloc:x}");

    let now_as_secs = {
        let now = SystemTime::now();
        now.duration_since(&SystemTime::UNIX_EPOCH)?.as_secs()
    };

    if tloc != 0 {
        ctx.user_space().write_val(tloc, &now_as_secs)?;
    }

    Ok(SyscallReturn::Return(now_as_secs as _))
}
