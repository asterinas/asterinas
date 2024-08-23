// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::signal::{
        constants::{SIGKILL, SIGSTOP},
        sig_mask::SigMask,
        Pauser,
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

    debug_assert!(sigmask_size == core::mem::size_of::<SigMask>());
    if sigmask_size != core::mem::size_of::<SigMask>() {
        return_errno_with_message!(Errno::EINVAL, "invalid sigmask size");
    }

    let sigmask = {
        let mut mask: SigMask = ctx.get_user_space().read_val(sigmask_addr)?;
        // It is not possible to block SIGKILL or SIGSTOP,
        // specifying these signals in mask has no effect.
        mask -= SIGKILL;
        mask -= SIGSTOP;
        mask
    };

    // Pause until receiving any signal
    let pauser = Pauser::new_with_mask(sigmask);
    pauser.pause_until(|| None::<()>)?;

    // This syscall should always return `Err(EINTR)`. This path should never be reached.
    unreachable!("rt_sigsuspend always return EINTR");
}
