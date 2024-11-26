// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_rt_sigpending(
    u_set_ptr: Vaddr,
    sigset_size: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "u_set_ptr = 0x{:x},  sigset_size = {}",
        u_set_ptr, sigset_size
    );
    if sigset_size != 8 {
        return_errno_with_message!(Errno::EINVAL, "sigset size is not equal to 8")
    }
    do_rt_sigpending(u_set_ptr, ctx)?;
    Ok(SyscallReturn::Return(0))
}

fn do_rt_sigpending(set_ptr: Vaddr, ctx: &Context) -> Result<()> {
    let combined_signals = {
        let sig_mask_value = ctx.posix_thread.sig_mask().load(Ordering::Relaxed);
        let sig_pending_value = ctx.posix_thread.sig_pending();
        sig_mask_value & sig_pending_value
    };

    ctx.user_space()
        .write_val(set_ptr, &u64::from(combined_signals))?;
    Ok(())
}
