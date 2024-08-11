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
    let user_heap = ctx.process.heap();
    let new_heap_end = user_heap.brk(new_heap_end)?;

    Ok(SyscallReturn::Return(new_heap_end as _))
}
