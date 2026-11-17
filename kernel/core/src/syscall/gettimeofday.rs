// SPDX-License-Identifier: MPL-2.0

use aster_time::read_system_time;
use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    prelude::*,
    time::{timeval_t, timezone_t},
};

pub(super) fn sys_gettimeofday(
    timeval_addr: Vaddr,
    timezone_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    if timeval_addr != 0 {
        let time_val = timeval_t::from(read_system_time().since_epoch());
        ctx.user_space().write_val(timeval_addr, &time_val)?;
    }

    if timezone_addr != 0 {
        // TODO: Return the actual system timezone when available.
        let tz = timezone_t::default();
        ctx.user_space().write_val(timezone_addr, &tz)?;
    }

    Ok(SyscallReturn::Return(0))
}
