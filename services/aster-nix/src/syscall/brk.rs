// SPDX-License-Identifier: MPL-2.0

use crate::{
    log_syscall_entry,
    prelude::*,
    syscall::{SyscallReturn, SYS_BRK},
};

/// expand the user heap to new heap end, returns the new heap end if expansion succeeds.
pub fn sys_brk(heap_end: u64) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_BRK);
    let new_heap_end = if heap_end == 0 {
        None
    } else {
        Some(heap_end as usize)
    };
    debug!("new heap end = {:x?}", heap_end);
    let current = current!();
    let user_heap = current.user_heap();
    let new_heap_end = user_heap.brk(new_heap_end)?;

    Ok(SyscallReturn::Return(new_heap_end as _))
}
