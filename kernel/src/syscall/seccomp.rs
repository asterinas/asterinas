// SPDX-License-Identifier: MPL-2.0

use ostd::{mm::VmIo, user::UserContextApi};

use super::{SyscallReturn, arch};
use crate::{
    prelude::*,
    process::{
        TermStatus,
        credentials::capabilities::CapSet,
        posix_thread::{do_exit, do_exit_group},
        seccomp::{
            AUDIT_ARCH_NATIVE, BPF_MAXINSNS, SECCOMP_GET_ACTION_AVAIL, SECCOMP_SET_MODE_FILTER,
            SECCOMP_SET_MODE_STRICT, SeccompAction, SeccompData, SeccompFilter, SeccompMode,
            SockFilter, is_action_available, validate_filter_flags,
        },
        signal::{
            c_types::siginfo_t,
            constants::{SIGKILL, SIGSYS, SYS_SECCOMP},
            signals::raw::RawSignal,
        },
    },
    security::lsm::hooks as lsm_hooks,
};

const MAX_ERRNO: u16 = 4095;
const SOCK_FPROG_FILTER_OFFSET: usize = 8;

pub fn sys_seccomp(
    operation: u32,
    flags: u32,
    args_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    match operation {
        SECCOMP_SET_MODE_STRICT => {
            if flags != 0 || args_addr != 0 {
                return_errno_with_message!(Errno::EINVAL, "invalid strict seccomp arguments");
            }
            ctx.posix_thread.enable_seccomp_strict()?;
            Ok(SyscallReturn::Return(0))
        }
        SECCOMP_SET_MODE_FILTER => {
            install_filter(flags, args_addr, ctx)?;
            Ok(SyscallReturn::Return(0))
        }
        SECCOMP_GET_ACTION_AVAIL => {
            if flags != 0 {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "invalid seccomp action-availability arguments"
                );
            }
            let action = ctx.user_space().read_val::<u32>(args_addr)?;
            if is_action_available(action) {
                Ok(SyscallReturn::Return(0))
            } else {
                return_errno_with_message!(Errno::EOPNOTSUPP, "unsupported seccomp action");
            }
        }
        _ => return_errno_with_message!(Errno::EINVAL, "unsupported seccomp operation"),
    }
}

/// Per `seccomp(2)`, a thread may install a filter only if it has set
/// `no_new_privs` or holds `CAP_SYS_ADMIN`; this keeps an unprivileged filter
/// from being used to gain privileges across a later `execve`.
fn may_install_filter(ctx: &Context) -> bool {
    if ctx.posix_thread.no_new_privs() {
        return true;
    }

    let user_ns = ctx.thread_local.borrow_user_ns();
    lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
        user_ns.as_ref(),
        ctx.posix_thread,
        CapSet::SYS_ADMIN,
    ))
    .is_ok()
}

pub fn install_filter(flags: u32, fprog_addr: Vaddr, ctx: &Context) -> Result<()> {
    validate_filter_flags(flags)?;
    if fprog_addr == 0 {
        return_errno_with_message!(Errno::EFAULT, "seccomp filter program pointer is NULL");
    }
    if !may_install_filter(ctx) {
        return_errno_with_message!(
            Errno::EACCES,
            "installing a seccomp filter requires CAP_SYS_ADMIN or no_new_privs"
        );
    }

    let len = ctx.user_space().read_val::<u16>(fprog_addr)?;
    let filter_ptr_addr = checked_user_addr_add(fprog_addr, SOCK_FPROG_FILTER_OFFSET)?;
    let filter_addr = ctx.user_space().read_val::<Vaddr>(filter_ptr_addr)?;
    if len == 0 {
        return_errno_with_message!(Errno::EINVAL, "empty seccomp filter program");
    }
    let len = len as usize;
    if len > BPF_MAXINSNS {
        return_errno_with_message!(Errno::EINVAL, "seccomp filter program is too large");
    }
    if filter_addr == 0 {
        return_errno_with_message!(Errno::EFAULT, "seccomp filter pointer is NULL");
    }

    let mut program = Vec::new();
    program.try_reserve_exact(len).map_err(|_| {
        Error::with_message(
            Errno::ENOMEM,
            "cannot allocate memory for seccomp filter program",
        )
    })?;

    let mut addr = filter_addr;
    for idx in 0..len {
        let insn = ctx.user_space().read_val::<SockFilter>(addr)?;
        program.push(insn);
        if idx + 1 != len {
            addr = checked_user_addr_add(addr, size_of::<SockFilter>())?;
        }
    }

    let filter = SeccompFilter::new(program.into_boxed_slice(), flags)?;
    ctx.posix_thread.append_seccomp_filter(filter)?;
    Ok(())
}

pub fn check(
    syscall_number: u64,
    args: &[u64; 6],
    ctx: &Context,
    user_ctx: &mut ostd::arch::cpu::context::UserContext,
) -> Result<Option<SyscallReturn>> {
    let mode = ctx.posix_thread.seccomp_mode();
    match mode {
        SeccompMode::Disabled => Ok(None),
        SeccompMode::Strict => Ok(check_strict(syscall_number, ctx, user_ctx)),
        SeccompMode::Filter => Ok(check_filter(syscall_number, args, ctx, user_ctx)),
    }
}

fn check_strict(
    syscall_number: u64,
    ctx: &Context,
    user_ctx: &mut ostd::arch::cpu::context::UserContext,
) -> Option<SyscallReturn> {
    if is_strict_allowed_syscall(syscall_number) {
        return None;
    }

    do_exit(TermStatus::Killed(SIGKILL), ctx, user_ctx);
    Some(SyscallReturn::NoReturn)
}

fn check_filter(
    syscall_number: u64,
    args: &[u64; 6],
    ctx: &Context,
    user_ctx: &mut ostd::arch::cpu::context::UserContext,
) -> Option<SyscallReturn> {
    let chain = ctx.posix_thread.seccomp_filter_chain()?;

    let data = SeccompData {
        nr: syscall_number as i32,
        arch: AUDIT_ARCH_NATIVE,
        instruction_pointer: user_ctx.instruction_pointer() as u64,
        args: *args,
    };

    let evaluation = chain.evaluate_with_metadata(&data);
    if evaluation.should_log || matches!(evaluation.action, SeccompAction::Log) {
        log_seccomp_action(ctx, syscall_number, evaluation.action);
    }

    match evaluation.action {
        SeccompAction::Allow => None,
        SeccompAction::Log => None,
        SeccompAction::Errno(errno) => Some(errno_action_return(errno)),
        SeccompAction::Trap(errno) => {
            let mut info = siginfo_t::new(SIGSYS, SYS_SECCOMP);
            info.si_errno = errno as i32;
            info.set_sigsys(
                user_ctx.instruction_pointer(),
                syscall_number as i32,
                AUDIT_ARCH_NATIVE,
            );
            ctx.posix_thread
                .enqueue_signal(Box::new(RawSignal::new(info)));
            Some(SyscallReturn::NoReturn)
        }
        SeccompAction::Trace(_) | SeccompAction::UserNotif(_) => Some(enosys_return()),
        SeccompAction::KillThread => {
            do_exit(TermStatus::Killed(SIGSYS), ctx, user_ctx);
            Some(SyscallReturn::NoReturn)
        }
        SeccompAction::KillProcess => {
            do_exit_group(TermStatus::Killed(SIGSYS), ctx, user_ctx);
            Some(SyscallReturn::NoReturn)
        }
    }
}

fn errno_action_return(errno: u16) -> SyscallReturn {
    let errno = errno.min(MAX_ERRNO) as i32;
    SyscallReturn::Return((-errno) as isize)
}

fn enosys_return() -> SyscallReturn {
    SyscallReturn::Return(-(Errno::ENOSYS as isize))
}

fn log_seccomp_action(ctx: &Context, syscall_number: u64, action: SeccompAction) {
    info!(
        "seccomp log: pid={}, tid={}, syscall={}, action={:?}",
        ctx.process.pid(),
        ctx.posix_thread.tid(),
        syscall_number,
        action
    );
}

fn checked_user_addr_add(addr: Vaddr, offset: usize) -> Result<Vaddr> {
    addr.checked_add(offset)
        .ok_or_else(|| Error::with_message(Errno::EFAULT, "user address overflows"))
}

fn is_strict_allowed_syscall(syscall_number: u64) -> bool {
    matches!(
        syscall_number,
        arch::SYS_READ | arch::SYS_WRITE | arch::SYS_EXIT | arch::SYS_RT_SIGRETURN
    )
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::*;

    #[ktest]
    fn seccomp() {
        seccomp_sock_fprog_layout_matches_linux_uapi();
        seccomp_strict_mode_uses_linux_allowlist();
        seccomp_errno_action_is_limited_to_max_errno();
        seccomp_sigsys_info_can_carry_seccomp_metadata();
    }

    #[ktest]
    fn seccomp_sock_fprog_layout_matches_linux_uapi() {
        let pointer_align = align_of::<Vaddr>();
        let expected_offset = size_of::<u16>().div_ceil(pointer_align) * pointer_align;
        assert_eq!(SOCK_FPROG_FILTER_OFFSET, expected_offset);
    }

    #[ktest]
    fn seccomp_strict_mode_uses_linux_allowlist() {
        assert!(is_strict_allowed_syscall(arch::SYS_READ));
        assert!(is_strict_allowed_syscall(arch::SYS_WRITE));
        assert!(is_strict_allowed_syscall(arch::SYS_EXIT));
        assert!(is_strict_allowed_syscall(arch::SYS_RT_SIGRETURN));
        assert!(!is_strict_allowed_syscall(arch::SYS_EXIT_GROUP));
    }

    #[ktest]
    fn seccomp_errno_action_is_limited_to_max_errno() {
        let SyscallReturn::Return(eperm) = errno_action_return(Errno::EPERM as u16) else {
            panic!("errno action must return to userspace");
        };
        let SyscallReturn::Return(max_errno) = errno_action_return(MAX_ERRNO + 1) else {
            panic!("errno action must return to userspace");
        };

        assert_eq!(eperm, -1);
        assert_eq!(max_errno, -4095);
    }

    #[ktest]
    fn seccomp_sigsys_info_can_carry_seccomp_metadata() {
        let mut info = siginfo_t::new(SIGSYS, SYS_SECCOMP);
        info.si_errno = Errno::EPERM as i32;
        info.set_sigsys(0x1234, arch::SYS_GETPID as i32, AUDIT_ARCH_NATIVE);

        assert_eq!(info.si_signo, SIGSYS.as_u8() as i32);
        assert_eq!(info.si_errno, Errno::EPERM as i32);
        assert_eq!(info.si_code, SYS_SECCOMP);
        assert_eq!(
            info.sigsys_fields(),
            (0x1234, arch::SYS_GETPID as i32, AUDIT_ARCH_NATIVE)
        );
    }
}
