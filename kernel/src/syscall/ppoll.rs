// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use super::{poll::do_sys_poll, SyscallReturn};
use crate::{
    prelude::*,
    process::signal::{sig_mask::SigMask, with_sigmask_changed},
    time::timespec_t,
};

pub fn sys_ppoll(
    fds: Vaddr,
    nfds: u32,
    timespec_addr: Vaddr,
    sigmask_addr: Vaddr,
    sigmask_size: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();

    let timeout = if timespec_addr != 0 {
        let time_spec = user_space.read_val::<timespec_t>(timespec_addr)?;
        Some(Duration::try_from(time_spec)?)
    } else {
        None
    };

    if sigmask_addr != 0 {
        if sigmask_size != size_of::<SigMask>() {
            return_errno_with_message!(Errno::EINVAL, "invalid sigmask size");
        }

        let sigmask = user_space.read_val::<SigMask>(sigmask_addr)?;
        with_sigmask_changed(ctx, |_| sigmask, || do_sys_poll(fds, nfds, timeout, ctx))
    } else {
        do_sys_poll(fds, nfds, timeout, ctx)
    }

    // TODO: Write back the remaining time to `timespec_addr`.
    //
    // The ppoll system call should write back the remaining time,
    // yet the function counterpart in libc hides this behavior to
    // make the API more portable across different UNIX-like OSes.
    // For the maximized Linux compatibility, we should follow Linux's behavior.
    // But this cannot be readily achieved given how our internal synchronization primitives
    // such as `Pause` and `WaitTimeout` work.
}
