use jinux_frame::cpu::UserContext;

use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::{clone_child, CloneArgs, CloneFlags};
use crate::util::read_val_from_user;

use super::{SyscallReturn, SYS_CLONE, SYS_CLONE3};

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
    let clone_args = CloneArgs::new(new_sp, 0, parent_tidptr, child_tidptr, tls, clone_flags);
    let child_pid = clone_child(parent_context, clone_args).unwrap();
    Ok(SyscallReturn::Return(child_pid as _))
}

pub fn sys_clone3(
    clong_args_addr: Vaddr,
    size: usize,
    parent_context: UserContext,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_CLONE3);
    trace!(
        "clone args addr = 0x{:x}, size = 0x{:x}",
        clong_args_addr,
        size
    );
    if size != core::mem::size_of::<Clone3Args>() {
        return_errno_with_message!(Errno::EINVAL, "invalid size");
    }

    let clone_args = {
        let args: Clone3Args = read_val_from_user(clong_args_addr)?;
        trace!("clone3 args = {:x?}", args);
        CloneArgs::from(args)
    };
    debug!("clone args = {:x?}", clone_args);

    let child_pid = clone_child(parent_context, clone_args)?;
    trace!("child pid = {}", child_pid);
    Ok(SyscallReturn::Return(child_pid as _))
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
struct Clone3Args {
    /// Flags bit mask
    flags: u64,
    /// Where to store PID file descriptor
    pidfd: u64,
    /// Where to store child TID in child's memory
    child_tid: u64,
    ///  Where to store child TID in parent's memory
    parent_tid: u64,
    /// Signal to deliver to parent on child termination
    exit_signal: u64,
    /// Pointer to lowest byte of stack
    stack: u64,
    /// Size of stack
    stack_size: u64,
    /// Location of new TLS
    tls: u64,
    /// Pointer to a pid_t array
    set_tid: u64,
    /// Number of elements in set_tid
    set_tid_size: u64,
    /// File descriptor for target cgroup of child
    cgroup: u64,
}

impl From<Clone3Args> for CloneArgs {
    fn from(value: Clone3Args) -> Self {
        const FLAGS_MASK: u64 = 0xff;
        let clone_flags =
            CloneFlags::from(value.exit_signal & FLAGS_MASK | value.flags & !FLAGS_MASK);
        // TODO: deal with pidfd, exit_signal, set_tid, set_tid_size, cgroup
        CloneArgs::new(
            value.stack,
            value.stack_size as _,
            value.parent_tid as _,
            value.child_tid as _,
            value.tls,
            clone_flags,
        )
    }
}
