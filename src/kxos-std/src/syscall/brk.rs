use crate::prelude::*;

use crate::{process::Process, syscall::SYS_BRK};

/// expand the user heap to new heap end, returns the new heap end if expansion succeeds.
pub fn sys_brk(heap_end: u64) -> Result<isize> {
    debug!("[syscall][id={}][SYS_BRK]", SYS_BRK);
    let current = Process::current();
    let new_heap_end = if heap_end == 0 {
        None
    } else {
        Some(heap_end as usize)
    };
    let current = Process::current();
    let user_heap = current
        .user_heap()
        .expect("brk should work on process with user heap");
    let vm_space = current
        .vm_space()
        .expect("brk should work on process with user space");
    let new_heap_end = user_heap.brk(new_heap_end, vm_space);

    Ok(new_heap_end as _)
}
