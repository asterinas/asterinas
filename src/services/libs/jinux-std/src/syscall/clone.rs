use jinux_frame::cpu::CpuContext;

use crate::process::clone::{clone_child, CloneArgs, CloneFlags};
use crate::{prelude::*, syscall::SYS_CLONE};

use super::SyscallReturn;

// The order of arguments for clone differs in different architecture.
// This order we use here is the order for x86_64. See https://man7.org/linux/man-pages/man2/clone.2.html.
pub fn sys_clone(
    clone_flags: u64,
    new_sp: Vaddr,
    parent_tidptr: Vaddr,
    child_tidptr: Vaddr,
    tls: usize,
    parent_context: CpuContext,
) -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_CLONE]", SYS_CLONE);
    let clone_flags = CloneFlags::from(clone_flags);
    debug!("flags = {:?}, child_stack_ptr = 0x{:x}, parent_tid_ptr = 0x{:x}, child tid ptr = 0x{:x}, tls = 0x{:x}", clone_flags, new_sp, parent_tidptr, child_tidptr, tls);
    let clone_args = CloneArgs::new(new_sp, parent_tidptr, child_tidptr, tls, clone_flags);
    let child_process = clone_child(parent_context, clone_args).unwrap();
    let child_pid = child_process.pid();
    let pid = current!().pid();
    debug!("*********schedule child process, pid = {}**********", pid);
    child_process.send_to_scheduler();
    debug!("*********return to parent process, pid = {}*********", pid);
    Ok(SyscallReturn::Return(child_pid as _))
}
