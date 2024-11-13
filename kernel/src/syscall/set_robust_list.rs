// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{prelude::*, process::posix_thread::RobustListHead};

pub fn sys_set_robust_list(
    robust_list_head_ptr: Vaddr,
    len: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
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
    let robust_list_head: RobustListHead = ctx.user_space().read_val(robust_list_head_ptr)?;
    debug!("{:x?}", robust_list_head);
    let mut robust_list = ctx.posix_thread.robust_list().lock();
    *robust_list = Some(robust_list_head);
    Ok(SyscallReturn::Return(0))
}
