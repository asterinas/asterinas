// SPDX-License-Identifier: MPL-2.0

use ostd::sync::Waiter;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::signal::{
        constants::{SIGKILL, SIGSTOP},
        sig_mask::SigMask,
        with_signal_blocked,
    },
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

    if sigmask_size != core::mem::size_of::<SigMask>() {
        return_errno_with_message!(Errno::EINVAL, "invalid sigmask size");
    }

    let sigmask = {
        let mut mask: SigMask = ctx.user_space().read_val(sigmask_addr)?;
        // It is not possible to block SIGKILL or SIGSTOP,
        // specifying these signals in mask has no effect.
        mask -= SIGKILL;
        mask -= SIGSTOP;
        mask
    };

    // Wait until receiving any signal
    let waiter = Waiter::new_pair().0;
    with_signal_blocked(ctx, sigmask, || waiter.pause_until(|| None::<()>))?;

    // This syscall should always return `Err(EINTR)`. This path should never be reached.
    unreachable!("rt_sigsuspend always return EINTR");
}
