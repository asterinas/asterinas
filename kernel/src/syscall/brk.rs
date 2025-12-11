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

    let syscall_ret = match new_heap_end {
        Some(addr) => user_heap
            .set_program_break(addr, ctx)
            .unwrap_or_else(|current_break| current_break),
        None => user_heap.program_break(),
    };

    Ok(SyscallReturn::Return(syscall_ret as _))
}
