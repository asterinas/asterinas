// SPDX-License-Identifier: MPL-2.0

use super::{
    SyscallReturn,
    rt_sigprocmask::{AllowTruncSize, UserSigSetPtr},
};
use crate::{prelude::*, process::signal::HandlePendingSignal};

pub fn sys_rt_sigpending(
    u_set_ptr: Vaddr,
    sigset_size: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "u_set_ptr = 0x{:x},  sigset_size = {}",
        u_set_ptr, sigset_size
    );
    let size_policy = AllowTruncSize::new(sigset_size)?;
    do_rt_sigpending(u_set_ptr, size_policy, ctx)?;
    Ok(SyscallReturn::Return(0))
}

fn do_rt_sigpending(set_ptr: Vaddr, size_policy: AllowTruncSize, ctx: &Context) -> Result<()> {
    let combined_signals = {
        let sig_mask_value = ctx.posix_thread.sig_mask();
        let sig_pending_value = ctx.pending_signals();
        sig_mask_value & sig_pending_value
    };

    let user_space = ctx.user_space();
    let set_ptr = UserSigSetPtr::new(&user_space, set_ptr, size_policy);
    set_ptr.write_val(&combined_signals)?;
    Ok(())
}
