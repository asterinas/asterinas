// SPDX-License-Identifier: MPL-2.0

use ostd::sync::Waiter;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{posix_thread::ContextPthreadAdminApi, signal::sig_mask::SigMask},
};

pub fn sys_rt_sigsuspend(
    sigmask_addr: Vaddr,
    sigmask_size: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "sigmask_addr = 0x{:x}, sigmask_size = {}",
        sigmask_addr, sigmask_size
    );

    let checked_size = SigMask::check_full_size(sigmask_size)?;

    let sigmask = checked_size.read_val(&ctx.user_space(), sigmask_addr)?;
    ctx.save_and_set_sig_mask(sigmask);

    // Wait until receiving any signal
    let waiter = Waiter::new_pair().0;
    waiter.pause_until(|| None::<()>)?;

    // This syscall should always return `Err(EINTR)`. This path should never be reached.
    unreachable!("rt_sigsuspend always return EINTR");
}
