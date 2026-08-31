use ostd::arch::cpu::context::UserContext;

use super::{SyscallArgument, SyscallReturn};
use crate::{
    cbpf::{
        ClassicBPFilter, NetFilterProg, RawFilterBlock, SeccompFilterLeaf, SeccompFilterProg,
        SeccompMode::{self},
        SeccompOp::{self},
        SeccompRet, UnverifiedFilterProg,
    },
    prelude::*,
    process::posix_thread::PosixThread,
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
            seccomp_set_mode_strict(ctx)
        }
        SeccompOp::SetModeFilter => seccomp_set_mode_filter(flags, uargs, ctx),
    }?;

    Ok(SyscallReturn::Return(res as _))
}

fn seccomp_set_mode_strict(ctx: &Context) -> Result<i64> {
    // Linux does mitigations here
    // filter to strict should be allowed, but we will need to drop the Arc reference so there's no memory leak

    seccomp_assign_mode(ctx.posix_thread, SeccompMode::Strict, 0)
}

fn seccomp_assign_mode(current: &PosixThread, mode: SeccompMode, _flags: u64) -> Result<i64> {
    // Linux does additional mitigations here (signal handling, no_new_privs, etc.).
    current.set_seccomp_mode(mode)
}

// Pointer to the filter program in user space.
#[derive(Clone, Copy, Pod)]
struct UserspaceFilterMeta {
    pub user_buf_ptr: Vaddr,
    pub user_buf_len: usize,
}

/// TODO check flags
fn seccomp_set_mode_filter(_flags: u32, uargs: Vaddr, ctx: &Context) -> Result<i64> {
    let filter_meta: UserspaceFilterMeta = ctx
        .user_space()
        .vmar()
        .vm_space()
        .reader(uargs, size_of::<UserspaceFilterMeta>())?
        .read_val()?;

    let filter_len = filter_meta.user_buf_len;
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

    let seccomp_state = ctx.posix_thread.seccomp_state();

    ctx.posix_thread
        .set_seccomp_filter(Arc::new(SeccompFilterLeaf {
            ins: seccompfilter,
            prev: seccomp_state.leaf_filter.clone(),
        }));

    if seccomp_state.mode != SeccompMode::Filter {
        return seccomp_assign_mode(ctx.posix_thread, SeccompMode::Filter, 0);
    }

    Ok(0)
}

pub(super) enum SeccompFilterAction {
    Allow,
    Errno(Errno),
    Kill,
    #[expect(dead_code)] // TODO
    Trace(u32),
}

pub(super) fn execute_seccomp_filter(
    ctx: &Context,
    user_ctx: &UserContext,
    syscall_frame: &SyscallArgument,
) -> Result<SeccompFilterAction> {
    let mut current = ctx.posix_thread.seccomp_filter();

    // Walk leaf → root, keeping the signed minimum across all filters.
    // A lower (more negative when cast to i32) return value wins.
    let mut result = SeccompRet::Allow as u32;
    while let Some(leaf) = current {
        let n = leaf
            .ins
            .execute(user_ctx, syscall_frame.syscall_number, &syscall_frame.args)?;
        result = (result as i32).min(n as i32) as u32;
        current = leaf.prev.clone();
    }

    parse_seccomp_return(result)
}

fn parse_seccomp_return(return_value: u32) -> Result<SeccompFilterAction> {
    use crate::cbpf::SECCOMP_RET_MASK;

    match (return_value & SECCOMP_RET_MASK).try_into()? {
        SeccompRet::Allow => Ok(SeccompFilterAction::Allow),
        SeccompRet::Errno => {
            let errno = (return_value & 0xffff) as i32;
            let errno = Errno::try_from(errno).map_err(|_| Error::new(Errno::EINVAL))?;
            Ok(SeccompFilterAction::Errno(errno))
        }
        SeccompRet::Kill => Ok(SeccompFilterAction::Kill),
        SeccompRet::Trace => Ok(SeccompFilterAction::Trace(return_value & 0xffff)),
        SeccompRet::Trap => Ok(SeccompFilterAction::Kill),
    }
}
