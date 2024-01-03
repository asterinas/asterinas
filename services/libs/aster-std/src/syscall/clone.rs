// SPDX-License-Identifier: MPL-2.0

use aster_frame::cpu::UserContext;

use crate::log_syscall_entry;
use crate::process::{clone_child, CloneArgs, CloneFlags};
use crate::{prelude::*, syscall::SYS_CLONE};

use super::SyscallReturn;

// The order of arguments for clone differs in different architecture.
// This order we use here is the order for x86_64. See https://man7.org/linux/man-pages/man2/clone.2.html.
pub fn sys_clone(
    clone_flags: u64,
    new_sp: u64,
    parent_tidptr: Vaddr,
    child_tidptr: Vaddr,
    tls: u64,
    parent_context: UserContext,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_CLONE);
    let clone_flags = CloneFlags::from(clone_flags);
    debug!("flags = {:?}, child_stack_ptr = 0x{:x}, parent_tid_ptr = 0x{:x}, child tid ptr = 0x{:x}, tls = 0x{:x}", clone_flags, new_sp, parent_tidptr, child_tidptr, tls);
    let clone_args = CloneArgs::new(new_sp, parent_tidptr, child_tidptr, tls, clone_flags);
    let child_pid = clone_child(parent_context, clone_args).unwrap();
    Ok(SyscallReturn::Return(child_pid as _))
}
