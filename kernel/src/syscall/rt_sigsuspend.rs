// SPDX-License-Identifier: MPL-2.0

use ostd::sync::Waiter;

use super::{
    SyscallReturn,
    rt_sigprocmask::{RequireFullSize, UserSigSetPtr},
};
use crate::{prelude::*, process::posix_thread::ContextPthreadAdminApi};

pub fn sys_rt_sigsuspend(
    sigmask_addr: Vaddr,
    sigmask_size: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "sigmask_addr = 0x{:x}, sigmask_size = {}",
        sigmask_addr, sigmask_size
    );

    let size_policy = RequireFullSize::new(sigmask_size)?;

    let user_space = ctx.user_space();
    let sigmask_ptr = UserSigSetPtr::new(&user_space, sigmask_addr, size_policy);
    let sigmask = sigmask_ptr.read_val()?;
    ctx.save_and_set_sig_mask(sigmask);

    // Wait until receiving any signal
    let waiter = Waiter::new_pair().0;
    waiter.pause_until(|| None::<()>)?;

    // This syscall should always return `Err(EINTR)`. This path should never be reached.
    unreachable!("rt_sigsuspend always return EINTR");
}
