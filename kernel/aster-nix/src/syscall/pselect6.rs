// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use super::{select::sys_select, SyscallReturn};
use crate::{
    fs::file_table::FileDesc,
    prelude::*,
    process::{posix_thread::PosixThreadExt, signal::sig_mask::SigMask},
    util::read_val_from_user,
};

pub fn sys_pselect6(
    nfds: FileDesc,
    readfds_addr: Vaddr,
    writefds_addr: Vaddr,
    exceptfds_addr: Vaddr,
    timeval_addr: Vaddr,
    sigmask_addr: Vaddr,
) -> Result<SyscallReturn> {
    let current_thread = current_thread!();
    let posix_thread = current_thread.as_posix_thread().unwrap();

    let old_simask = if sigmask_addr != 0 {
        let new_sigmask: SigMask = read_val_from_user(sigmask_addr)?;
        let old_sigmask = posix_thread.sig_mask().swap(new_sigmask, Ordering::Relaxed);

        Some(old_sigmask)
    } else {
        None
    };

    let res = sys_select(
        nfds,
        readfds_addr,
        writefds_addr,
        exceptfds_addr,
        timeval_addr,
    );

    if let Some(old_mask) = old_simask {
        posix_thread.sig_mask().store(old_mask, Ordering::Relaxed);
    }

    res
}
