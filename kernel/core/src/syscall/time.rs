// SPDX-License-Identifier: MPL-2.0

use aster_time::read_system_time;
use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::prelude::*;

pub(super) fn sys_time(tloc: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    debug!("tloc = 0x{tloc:x}");

    let now_as_secs = read_system_time().since_epoch().as_secs();

    if tloc != 0 {
        ctx.user_space().write_val(tloc, &now_as_secs)?;
    }

    Ok(SyscallReturn::Return(now_as_secs as _))
}
