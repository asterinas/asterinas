// SPDX-License-Identifier: MPL-2.0

use core::{sync::atomic::Ordering, time::Duration};

use super::{select::do_sys_select, SyscallReturn};
use crate::{
    fs::file_table::FileDesc, prelude::*, process::signal::sig_mask::SigMask, time::timespec_t,
};

pub fn sys_pselect6(
    nfds: FileDesc,
    readfds_addr: Vaddr,
    writefds_addr: Vaddr,
    exceptfds_addr: Vaddr,
    timespec_addr: Vaddr,
    sigmask_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let old_simask = if sigmask_addr != 0 {
        let sigmask_with_size: SigMaskWithSize = user_space.read_val(sigmask_addr)?;

        if !sigmask_with_size.is_valid() {
            return_errno_with_message!(Errno::EINVAL, "sigmask size is invalid")
        }
        let old_sigmask = ctx
            .posix_thread
            .sig_mask()
            .swap(sigmask_with_size.sigmask, Ordering::Relaxed);

        Some(old_sigmask)
    } else {
        None
    };

    let timeout = if timespec_addr != 0 {
        let time_spec: timespec_t = user_space.read_val(timespec_addr)?;
        Some(Duration::try_from(time_spec)?)
    } else {
        None
    };

    let res = do_sys_select(
        nfds,
        readfds_addr,
        writefds_addr,
        exceptfds_addr,
        timeout,
        ctx,
    );

    if let Some(old_mask) = old_simask {
        ctx.posix_thread
            .sig_mask()
            .store(old_mask, Ordering::Relaxed);
    }

    res
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
struct SigMaskWithSize {
    sigmask: SigMask,
    sigmasksize: usize,
}

impl SigMaskWithSize {
    const fn is_valid(&self) -> bool {
        self.sigmask.is_empty() || self.sigmasksize == size_of::<SigMask>()
    }
}
