// SPDX-License-Identifier: MPL-2.0

use core::num::NonZeroU64;

use ostd::{arch::cpu::context::UserContext, mm::VmIo};

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{CloneArgs, CloneFlags, clone_child, signal::sig_num::SigNum},
    vm::vmar::is_userspace_vaddr,
};

// The order of arguments for clone differs in different architecture.
// This order we use here is the order for x86_64. See https://man7.org/linux/man-pages/man2/clone.2.html.
pub fn sys_clone(
    clone_flags: u64,
    new_sp: u64,
    parent_tidptr: Vaddr,
    child_tidptr: Vaddr,
    tls: u64,
    ctx: &Context,
    parent_context: &UserContext,
) -> Result<SyscallReturn> {
    let args = CloneArgs::for_clone(clone_flags, parent_tidptr, child_tidptr, tls, new_sp)?;
    debug!("clone args = {:x?}", args);

    let child_pid = clone_child(ctx, parent_context, args)?;
    Ok(SyscallReturn::Return(child_pid as _))
}

pub fn sys_clone3(
    clong_args_addr: Vaddr,
    size: usize,
    ctx: &Context,
    parent_context: &UserContext,
) -> Result<SyscallReturn> {
    trace!(
        "clone args addr = 0x{:x}, size = 0x{:x}",
        clong_args_addr, size
    );
    if size != size_of::<Clone3Args>() {
        return_errno_with_message!(Errno::EINVAL, "invalid size");
    }

    let clone_args = {
        let args: Clone3Args = ctx.user_space().read_val(clong_args_addr)?;
        trace!("clone3 args = {:x?}", args);
        CloneArgs::try_from(args)?
    };
    debug!("clone args = {:x?}", clone_args);

    let child_pid = clone_child(ctx, parent_context, clone_args)?;
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

impl TryFrom<Clone3Args> for CloneArgs {
    type Error = Error;

    fn try_from(value: Clone3Args) -> Result<Self> {
        // TODO: Deal with set_tid, set_tid_size, cgroup
        if value.set_tid != 0 || value.set_tid_size != 0 {
            warn!("set_tid is not supported");
        }
        if value.cgroup != 0 {
            warn!("cgroup is not supported");
        }

        // This checks arguments only for the `clone3()` system call.
        // Reference: <https://elixir.bootlin.com/linux/v6.16.9/source/kernel/fork.c#L2843-L2869>.

        let flags = CloneFlags::from_bits(value.flags as u32)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid clone flags"))?;
        let exit_signal =
            (value.exit_signal != 0).then(|| SigNum::from_u8(value.exit_signal as u8));
        if flags.intersects(CloneFlags::CLONE_PARENT | CloneFlags::CLONE_THREAD)
            && exit_signal.is_some()
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "`CLONE_PARENT` and  `CLONE_THREAD` cannot be used if the exit signal is specified"
            )
        }

        let (stack, stack_size) = match (
            NonZeroU64::new(value.stack),
            NonZeroU64::new(value.stack_size),
        ) {
            (Some(_), None) | (None, Some(_)) => return_errno_with_message!(
                Errno::EINVAL,
                "the stack and the stack size must be specified together"
            ),
            (Some(stack), Some(stack_size))
                if !is_userspace_vaddr(stack.get() as Vaddr)
                    || stack
                        .checked_add(stack_size.get() - 1)
                        .is_none_or(|stack_end| !is_userspace_vaddr(stack_end.get() as Vaddr)) =>
            {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "the stack address is not in the userspace"
                )
            }
            vals => vals,
        };

        Ok(Self {
            flags,
            pidfd: Some(value.pidfd as Vaddr),
            child_tid: value.child_tid as _,
            parent_tid: Some(value.parent_tid as _),
            exit_signal,
            stack,
            stack_size,
            tls: value.tls,
            _set_tid: Some(value.set_tid),
            _set_tid_size: Some(value.set_tid_size),
            _cgroup: Some(value.cgroup),
        })
    }
}
