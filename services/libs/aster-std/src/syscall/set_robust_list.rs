// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_SET_ROBUST_LIST};
use crate::{
    log_syscall_entry,
    prelude::*,
    process::posix_thread::{PosixThreadExt, RobustListHead},
    util::read_val_from_user,
};

pub fn sys_set_robust_list(robust_list_head_ptr: Vaddr, len: usize) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SET_ROBUST_LIST);
    debug!(
        "robust list head ptr: 0x{:x}, len = {}",
        robust_list_head_ptr, len
    );
    if len != core::mem::size_of::<RobustListHead>() {
        return_errno_with_message!(
            Errno::EINVAL,
            "The len is not equal to the size of robust list head"
        );
    }
    let robust_list_head: RobustListHead = read_val_from_user(robust_list_head_ptr)?;
    debug!("{:x?}", robust_list_head);
    let current_thread = current_thread!();
    let posix_thread = current_thread.as_posix_thread().unwrap();
    let mut robust_list = posix_thread.robust_list().lock();
    *robust_list = Some(robust_list_head);
    Ok(SyscallReturn::Return(0))
}
