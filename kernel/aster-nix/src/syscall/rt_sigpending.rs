// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use super::SyscallReturn;
use crate::{prelude::*, process::posix_thread::PosixThreadExt, util::write_val_to_user};

pub fn sys_rt_sigpending(u_set_ptr: Vaddr, sigset_size: usize) -> Result<SyscallReturn> {
    debug!(
        "u_set_ptr = 0x{:x},  sigset_size = {}",
        u_set_ptr, sigset_size
    );
    if sigset_size != 8 {
        return_errno_with_message!(Errno::EINVAL, "sigset size is not equal to 8")
    }
    do_rt_sigpending(u_set_ptr, sigset_size)?;
    Ok(SyscallReturn::Return(0))
}

fn do_rt_sigpending(set_ptr: Vaddr, sigset_size: usize) -> Result<()> {
    let current_thread = current_thread!();
    let posix_thread = current_thread.as_posix_thread().unwrap();

    let combined_signals = {
        let sig_mask_value = posix_thread.sig_mask().lock().as_u64();
        let sig_pending_value = posix_thread.sig_pending().as_u64();
        sig_mask_value & sig_pending_value
    };

    write_val_to_user(set_ptr, &combined_signals)?;
    Ok(())
}
