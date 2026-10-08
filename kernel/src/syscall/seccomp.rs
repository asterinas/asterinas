use ostd::arch::cpu::context::UserContext;

use super::{SyscallArgument, SyscallReturn};
use crate::{
    prelude::*,
    process::{
        credentials::capabilities::CapSet,
        posix_thread::{
            AsPosixThread, PosixThread,
            cbpf::{
                self, ClassicBPFilter, NetFilterProg, RawFilterBlock, SeccompContext,
                SeccompFilterLeaf, SeccompFilterProg,
                SeccompMode::{self},
                SeccompOp::{self},
                SeccompRet, UnverifiedFilterProg,
                cbpf_opcodes::BPF_MAXINS,
            },
        },
    },
    thread::Tid,
};

pub fn sys_seccomp(op: u64, flags: u32, uargs: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let op = match op {
        0 => SeccompOp::SetModeStrict,
        1 => SeccompOp::SetModeFilter,
        _ => Err(Error::new(Errno::EINVAL))?,
    };

    do_seccomp(op, flags, uargs, ctx)
}

fn do_seccomp(op: SeccompOp, flags: u32, uargs: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let res: i64 = match op {
        SeccompOp::SetModeStrict => {
            if flags != 0 || uargs != 0 {
                return Err(Error::new(Errno::EINVAL));
            }
            ctx.posix_thread.set_seccomp_strict()
        }
        SeccompOp::SetModeFilter => seccomp_set_mode_filter(flags, uargs, ctx),
    }?;

    Ok(SyscallReturn::Return(res as _))
}

fn is_ancestor(
    mut caller_leaf: Option<&SeccompFilterLeaf>,
    target_leaf: Option<&SeccompFilterLeaf>,
) -> bool {
    let Some(target) = target_leaf else {
        // If target has no filter, an empty filter chain is trivially an ancestor
        return true;
    };

    // Target has a filter; walk caller's ancestors to see if target's root matches
    while let Some(caller) = caller_leaf {
        if core::ptr::eq(caller, target) {
            return true;
        }
        caller_leaf = caller.prev.as_deref();
    }

    false
}

fn seccomp_sync_threads(ctx: &Context, new_filter: SeccompFilterProg) -> Result<(), Tid> {
    let tasks_guard = ctx.process.tasks().lock();
    let current_thread = ctx.posix_thread;
    let current_state = current_thread.seccomp_state();

    // Phase 1: Validation
    for task in tasks_guard.as_slice() {
        let Some(posix_thread) = task.as_posix_thread() else {
            continue;
        };
        if core::ptr::eq(posix_thread, current_thread) {
            continue;
        }

        let other_state = posix_thread.seccomp_state();
        let can_sync = match other_state.mode() {
            SeccompMode::Disabled => true,
            SeccompMode::Filter => {
                is_ancestor(current_state.leaf_filter(), other_state.leaf_filter())
            }
            SeccompMode::Strict => false,
        };

        if !can_sync {
            return Err(posix_thread.tid());
        }
    }

    let new_state = current_thread.set_n_push_seccomp_filter(new_filter);

    // Update all sibling threads
    let no_new_privs = current_thread.no_new_privs();
    for task in tasks_guard.as_slice() {
        let Some(thread) = task.as_posix_thread() else {
            continue;
        };
        if no_new_privs {
            thread.set_no_new_privs();
        }
        thread.set_seccomp_state(new_state.clone());
    }

    Ok(())
}

// Pointer to the filter program in user space.
#[repr(C)]
#[derive(Clone, Copy, Pod)]
struct UserspaceFilterMeta {
    user_buf_len: u16,
    _pad: [u8; 6],
    user_buf_ptr: Vaddr,
}

bitflags! {
    /// Flags for `SeccompOp::SetModeFilter`.
    pub struct SeccompFilterFlags: u32 {
        /// Synchronize all other threads to the same filter tree.
        const TSYNC = 1 << 0;
        /// All filter returns except `ALLOW` should be logged.
        const LOG = 1 << 1;
        /// Disable Speculative Store Bypass mitigations.
        const SPEC_ALLOW = 1 << 2;
        /// Return a new user-space listener file descriptor.
        const NEW_LISTENER = 1 << 3;
        /// Return -ESRCH when TSYNC fails instead of thread ID.
        const TSYNC_ESRCH = 1 << 4;
        /// Allow killable wait for user notifications.
        const WAIT_KILLABLE_RECV = 1 << 5;
    }
}

fn seccomp_set_mode_filter(flags_raw: u32, uargs: Vaddr, ctx: &Context) -> Result<i64> {
    let flags = SeccompFilterFlags::from_bits(flags_raw)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown seccomp filter flags"))?;

    // TODO implement the rest, remove this check
    const SUPPORTED: SeccompFilterFlags =
        SeccompFilterFlags::TSYNC.union(SeccompFilterFlags::TSYNC_ESRCH);
    if !SUPPORTED.contains(flags) {
        return_errno_with_message!(Errno::EINVAL, "unsupported seccomp filter flags");
    }

    let thread = ctx.posix_thread;

    if !thread
        .credentials()
        .effective_capset()
        .contains(CapSet::SYS_ADMIN)
        && !thread.no_new_privs()
    {
        return Err(Error::new(Errno::EACCES));
    }

    if flags.contains(SeccompFilterFlags::TSYNC_ESRCH) && !flags.contains(SeccompFilterFlags::TSYNC)
    {
        // TSYNC_ESRCH requires TSYNC
        return Err(Error::new(Errno::EINVAL));
    }

    let filter_meta: UserspaceFilterMeta = ctx
        .user_space()
        .vmar()
        .vm_space()
        .reader(uargs, size_of::<UserspaceFilterMeta>())?
        .read_val()?;

    let filter_len = filter_meta.user_buf_len;
    if filter_len == 0 || filter_len > BPF_MAXINS {
        return_errno_with_message!(Errno::EINVAL, "invalid seccomp filter length");
    }

    let mut insns = UnverifiedFilterProg::new(filter_len);

    for i in 0..filter_len {
        let raw_instruction = ctx
            .user_space()
            .vmar()
            .vm_space()
            .reader(
                filter_meta.user_buf_ptr + size_of::<RawFilterBlock>() * i,
                size_of::<RawFilterBlock>(),
            )?
            .read_val::<RawFilterBlock>()?;

        insns.push(raw_instruction);
    }

    let netfilter = NetFilterProg::from_unverified(insns)?;
    let seccompfilter = SeccompFilterProg::from_netfilter(netfilter)?;

    if flags.contains(SeccompFilterFlags::TSYNC) {
        match seccomp_sync_threads(ctx, seccompfilter) {
            Ok(()) => return Ok(0),
            Err(failed_tid) => {
                if flags.contains(SeccompFilterFlags::TSYNC_ESRCH) {
                    return_errno!(Errno::ESRCH);
                } else {
                    return Ok(failed_tid as i64);
                }
            }
        }
    } else {
        thread.set_n_push_seccomp_filter(seccompfilter);
        Ok(0)
    }
}

/// Action to be taken by the hypervisor based on seccomp filter result
pub(super) enum SeccompFilterAction {
    KillProcess,
    KillThread,
    Trap(u16),
    Errno(u16),
    UserNotif,
    Log,
    Trace(#[expect(dead_code)] u32),
    Allow,
}

const SECCOMP_RET_ACTION: u32 = 0xffff_0000;
#[inline]
fn action_only(ret: u32) -> i32 {
    (ret & SECCOMP_RET_ACTION) as i32
}

pub(super) fn execute_seccomp_filter(
    posix_thread: &PosixThread,
    user_ctx: &UserContext,
    syscall_frame: &SyscallArgument,
) -> Result<SeccompFilterAction> {
    // Walk leaf → root, keeping the signed minimum across all filters.
    // A lower (more negative when cast to i32) return value wins.
    // TODO early exit on kill?
    let mut result = SeccompRet::Allow as u32;
    for leaf in posix_thread.seccomp_state().into_iter() {
        let n = leaf.ins.execute(SeccompContext::new(
            user_ctx,
            syscall_frame.syscall_number,
            &syscall_frame.args,
        ))?;

        if action_only(n) < action_only(result) {
            result = n;
        }
    }

    parse_seccomp_return(result)
}

fn parse_seccomp_return(return_value: u32) -> Result<SeccompFilterAction> {
    use cbpf::SECCOMP_RET_MASK;

    match (return_value & SECCOMP_RET_MASK).try_into() {
        Ok(SeccompRet::KillProcess) => Ok(SeccompFilterAction::KillProcess),
        Ok(SeccompRet::KillThread) => Ok(SeccompFilterAction::KillThread),
        Ok(SeccompRet::Trap) => Ok(SeccompFilterAction::Trap((return_value & 0xffff) as u16)),
        Ok(SeccompRet::Errno) => Ok(SeccompFilterAction::Errno((return_value & 0xffff) as u16)),
        Ok(SeccompRet::UserNotif) => Ok(SeccompFilterAction::UserNotif),
        Ok(SeccompRet::Trace) => Ok(SeccompFilterAction::Trace(return_value & 0xffff)),
        Ok(SeccompRet::Log) => Ok(SeccompFilterAction::Log),
        Ok(SeccompRet::Allow) => Ok(SeccompFilterAction::Allow),
        Err(_) => Err(Error::new(Errno::EINVAL)),
    }
}
