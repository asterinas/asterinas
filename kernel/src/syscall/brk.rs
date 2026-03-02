// SPDX-License-Identifier: MPL-2.0

use crate::{prelude::*, syscall::SyscallReturn};

/// expand the user heap to new heap end, returns the new heap end if expansion succeeds.
pub fn sys_brk(heap_end: u64, ctx: &Context) -> Result<SyscallReturn> {
    let new_heap_end = if heap_end == 0 {
        None
    } else {
        Some(heap_end as usize)
    };
    debug!("new heap end = {:x?}", heap_end);

    let user_space = ctx.user_space();
    let user_heap = user_space.vmar().process_vm().heap();

    let current_heap_end = match new_heap_end {
        Some(addr) => user_heap
            .modify_heap_end(addr, ctx)
            .unwrap_or_else(|cur_heap_end| cur_heap_end),
        None => user_heap.lock().heap_range().end,
    };

    Ok(SyscallReturn::Return(current_heap_end as _))
}
