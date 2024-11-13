// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    time::{timeval_t, SystemTime},
};

// The use of the timezone structure is obsolete.
// Glibc sets the timezone_addr argument to NULL, so just ignore it.
pub fn sys_gettimeofday(
    timeval_addr: Vaddr,
    /* timezone_addr: Vaddr, */ ctx: &Context,
) -> Result<SyscallReturn> {
    if timeval_addr == 0 {
        return Ok(SyscallReturn::Return(0));
    }

    let time_val = {
        let now = SystemTime::now();
        let time_duration = now.duration_since(&SystemTime::UNIX_EPOCH)?;
        timeval_t::from(time_duration)
    };
    ctx.user_space().write_val(timeval_addr, &time_val)?;

    Ok(SyscallReturn::Return(0))
}
