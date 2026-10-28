// SPDX-License-Identifier: MPL-2.0

//! Per-thread Linux seccomp modes and filters.
//!
//! Installation copies and validates a cBPF program before publishing an immutable
//! filter chain. Syscall entry evaluates a stable snapshot, and fork/clone share
//! that chain while retaining independent per-thread heads.

macro_rules! __log_prefix {
    () => {
        "seccomp: "
    };
}

mod abi;
mod cbpf;
mod filter;
mod install;
mod listener;

#[cfg(ktest)]
mod test;

use ostd::{arch::cpu::context::UserContext, mm::VmIo};

use self::abi::{
    MODE_FILTER, MODE_STRICT, RET_ALLOW, RET_ERRNO, RET_KILL_PROCESS, RET_KILL_THREAD, RET_LOG,
    RET_TRACE, RET_TRAP, RET_USER_NOTIF, SeccompNotif, SeccompNotifResp, SeccompNotifSizes,
};
pub(crate) use self::{
    abi::SeccompData,
    filter::{SeccompDecision, SeccompState, ThreadSeccompState},
};
use crate::{cpu::LinuxAbi, prelude::*, process::signal};

const SET_MODE_STRICT: u32 = 0;
const SET_MODE_FILTER: u32 = 1;
const GET_ACTION_AVAIL: u32 = 2;
const GET_NOTIF_SIZES: u32 = 3;

/// Handles the supported seccomp operations.
pub(crate) fn seccomp(
    operation: u32,
    flags: u32,
    args_addr: Vaddr,
    ctx: &Context,
) -> Result<isize> {
    match operation {
        SET_MODE_STRICT => {
            if flags != 0 || args_addr != 0 {
                return_errno_with_message!(Errno::EINVAL, "invalid strict seccomp arguments");
            }
            install::install_strict(ctx).map(|()| 0)
        }
        SET_MODE_FILTER => install::install_filter(ctx, flags, args_addr),
        GET_NOTIF_SIZES => {
            if flags != 0 {
                return_errno_with_message!(Errno::EINVAL, "invalid seccomp query flags");
            }
            ctx.user_space().write_val(
                args_addr,
                &SeccompNotifSizes {
                    seccomp_notif: size_of::<SeccompNotif>() as u16,
                    seccomp_notif_resp: size_of::<SeccompNotifResp>() as u16,
                    seccomp_data: size_of::<SeccompData>() as u16,
                },
            )?;
            Ok(0)
        }
        GET_ACTION_AVAIL => {
            if flags != 0 {
                return_errno_with_message!(Errno::EINVAL, "invalid seccomp query flags");
            }
            let action: u32 = ctx.user_space().read_val(args_addr)?;
            match action {
                RET_ALLOW | RET_ERRNO | RET_KILL_PROCESS | RET_KILL_THREAD | RET_TRAP
                | RET_TRACE | RET_LOG | RET_USER_NOTIF => Ok(0),
                _ => return_errno_with_message!(Errno::EOPNOTSUPP, "unsupported seccomp action"),
            }
        }
        _ => return_errno_with_message!(Errno::EINVAL, "unsupported seccomp operation"),
    }
}

/// Delivers a seccomp trap with the syscall's original return-register value.
pub(crate) fn trap(ctx: &Context, user_ctx: &mut UserContext, syscall_number: i32, reason: u32) {
    let info = SeccompData::from_context(user_ctx, syscall_number).trap_info(reason);
    // Linux syscall_rollback() restores orig_ax on x86 and orig_a0 on RISC-V.
    // The original return register was saved before ptrace and syscall dispatch.
    user_ctx.set_syscall_ret(ctx.thread_local.orig_syscall_ret().unwrap_or(usize::MAX));
    signal::force_signal(info, ctx);
}

/// Handles `PR_SET_SECCOMP` using the same installation path as `seccomp(2)`.
pub(crate) fn prctl_set_seccomp(mode: u64, fprog_addr: Vaddr, ctx: &Context) -> Result<()> {
    // Unlike seccomp(2), prctl ignores its filter argument for strict mode.
    if mode == MODE_STRICT as u64 {
        return install::install_strict(ctx);
    }
    if mode != MODE_FILTER as u64 {
        return_errno_with_message!(Errno::EINVAL, "unsupported seccomp mode");
    }
    install::install_filter(ctx, 0, fprog_addr).map(|_| ())
}
