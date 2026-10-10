// SPDX-License-Identifier: MPL-2.0

//! `clock_settime(2)` and `settimeofday(2)`.
//!
//! Only `CLOCK_REALTIME` can be set. The call moves the wall clock by
//! storing an offset against the firmware's boot time; monotonic clocks
//! are unaffected (see `time::set_realtime`). Requires `CAP_SYS_TIME`.

use core::time::Duration;

use ostd::mm::VmIo;

use super::{ClockId, SyscallReturn};
use crate::{
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet},
    security::lsm::hooks as lsm_hooks,
    time::{clockid_t, set_realtime, timespec_t, timeval_t},
};

fn check_sys_time(ctx: &Context) -> Result<()> {
    lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
        UserNamespace::get_init_singleton().as_ref(),
        ctx.posix_thread,
        CapSet::SYS_TIME,
    ))
}

pub(super) fn sys_clock_settime(
    clockid: clockid_t,
    timespec_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let clock_id = ClockId::try_from(clockid)
        .map_err(|_| Error::with_message(Errno::EINVAL, "unknown clock id"))?;
    if clock_id != ClockId::CLOCK_REALTIME {
        return_errno_with_message!(Errno::EINVAL, "only CLOCK_REALTIME can be set");
    }
    check_sys_time(ctx)?;
    let timespec: timespec_t = ctx.user_space().read_val(timespec_addr)?;
    let new_now = Duration::try_from(timespec)?;
    debug!("clock_settime: CLOCK_REALTIME = {:?}", new_now);
    set_realtime(new_now)?;
    Ok(SyscallReturn::Return(0))
}

pub(super) fn sys_settimeofday(
    timeval_addr: Vaddr,
    _timezone_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    check_sys_time(ctx)?;
    if timeval_addr == 0 {
        // Only the (ignored) timezone was passed.
        return Ok(SyscallReturn::Return(0));
    }
    let timeval: timeval_t = ctx.user_space().read_val(timeval_addr)?;
    let new_now = Duration::try_from(timeval)?;
    debug!("settimeofday: {:?}", new_now);
    set_realtime(new_now)?;
    Ok(SyscallReturn::Return(0))
}
