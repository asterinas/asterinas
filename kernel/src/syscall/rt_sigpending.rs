// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::signal::{
        HandlePendingSignal,
        sig_mask::{SigMask, SigMaskTruncSize},
    },
};

pub fn sys_rt_sigpending(
    u_set_ptr: Vaddr,
    sigset_size: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "u_set_ptr = 0x{:x},  sigset_size = {}",
        u_set_ptr, sigset_size
    );
    let checked_size = SigMask::check_trunc_size(sigset_size)?;
    do_rt_sigpending(u_set_ptr, checked_size, ctx)?;
    Ok(SyscallReturn::Return(0))
}

fn do_rt_sigpending(set_ptr: Vaddr, checked_size: SigMaskTruncSize, ctx: &Context) -> Result<()> {
    let combined_signals = {
        let sig_mask_value = ctx.posix_thread.sig_mask();
        let sig_pending_value = ctx.pending_signals();
        sig_mask_value & sig_pending_value
    };

    checked_size.write_val(&ctx.user_space(), set_ptr, &combined_signals)?;
    Ok(())
}
