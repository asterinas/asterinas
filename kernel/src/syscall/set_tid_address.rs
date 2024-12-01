// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_set_tid_address(tidptr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    debug!("tidptr = 0x{:x}", tidptr);

    let clear_child_tid = ctx.thread_local.clear_child_tid().get();
    if clear_child_tid != 0 {
        // According to manuals at https://man7.org/linux/man-pages/man2/set_tid_address.2.html
        // We need to write 0 to clear_child_tid and do futex wake
        todo!()
    }

    ctx.thread_local.set_child_tid().set(clear_child_tid);

    let tid = ctx.posix_thread.tid();
    Ok(SyscallReturn::Return(tid as _))
}
