// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_seccomp(
    operation: u32,
    flags: u32,
    uargs: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let operation = SeccompOperation::from_bits(operation)
        .ok_or(Error::with_message(Errno::EINVAL, "invalid operation"))?;

    let flags = SeccompFlags::from_bits(flags)
        .ok_or(Error::with_message(Errno::EINVAL, "invalid flags"))?;

    match operation {
        SeccompOperation::SECCOMP_SET_MODE_STRICT => seccomp_set_mode_strict(ctx),
        SeccompOperation::SECCOMP_SET_MODE_FILTER => seccomp_set_mode_filter(flags, uargs, ctx),
        SeccompOperation::SECCOMP_GET_ACTION_AVAIL => seccomp_get_action_avail(uargs, ctx),
        SeccompOperation::SECCOMP_GET_NOTIF_SIZES => seccomp_get_notif_sizes(uargs, ctx),
        _ => return_errno_with_message!(Errno::EINVAL, "invalid operation"),
    }
}

fn seccomp_set_mode_strict(_ctx: &Context) -> Result<SyscallReturn> {
    return_errno_with_message!(Errno::EINVAL, "not implemented");
}

fn seccomp_set_mode_filter(
    _flags: SeccompFlags,
    _uargs: Vaddr,
    _ctx: &Context,
) -> Result<SyscallReturn> {
    return_errno_with_message!(Errno::EINVAL, "not implemented");
}

fn seccomp_get_action_avail(uargs: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let action = user_space.read_val::<u32>(uargs)?;

    if action == SeccompAction::SECCOMP_RET_KILL_PROCESS.bits()
        || action == SeccompAction::SECCOMP_RET_KILL_THREAD.bits()
        || action == SeccompAction::SECCOMP_RET_TRAP.bits()
        || action == SeccompAction::SECCOMP_RET_ERRNO.bits()
        || action == SeccompAction::SECCOMP_RET_USER_NOTIF.bits()
        || action == SeccompAction::SECCOMP_RET_TRACE.bits()
        || action == SeccompAction::SECCOMP_RET_LOG.bits()
        || action == SeccompAction::SECCOMP_RET_ALLOW.bits()
    {
        return Ok(SyscallReturn::Return(0));
    }

    return_errno_with_message!(Errno::EOPNOTSUPP, "action not supported");
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
struct SeccompNotifSizes {
    seccomp_notif: u16,
    seccomp_notif_resp: u16,
    seccomp_data: u16,
}

fn seccomp_get_notif_sizes(usizes: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    // TODO: Remove the dummy implementation and correctly implement this functionality.
    let sizes = SeccompNotifSizes {
        seccomp_notif: 0,
        seccomp_notif_resp: 0,
        seccomp_data: 0,
    };
    user_space.write_val(usizes, &sizes)?;
    Ok(SyscallReturn::Return(0))
}

bitflags! {
    /// Valid operations for seccomp syscall.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.15/source/include/uapi/linux/seccomp.h#L15>.
    struct SeccompOperation: u32 {
        const SECCOMP_SET_MODE_STRICT  = 0;
        const SECCOMP_SET_MODE_FILTER  = 1;
        const SECCOMP_GET_ACTION_AVAIL = 2;
        const SECCOMP_GET_NOTIF_SIZES  = 3;
    }
}

bitflags! {
    /// Valid flags for SECCOMP_SET_MODE_FILTER.
    ///
    /// Reference: https://elixir.bootlin.com/linux/v6.15/source/include/uapi/linux/seccomp.h#L21
    struct SeccompFlags: u32 {
        const SECCOMP_FILTER_FLAG_TSYNC             = 1 << 0;
        const SECCOMP_FILTER_FLAG_LOG               = 1 << 1;
        const SECCOMP_FILTER_FLAG_SPEC_ALLOW        = 1 << 2;
        const SECCOMP_FILTER_FLAG_NEW_LISTENER      = 1 << 3;
        const SECCOMP_FILTER_FLAG_TSYNC_ESRCH       = 1 << 4;
        const SECCOMP_FILTER_FLAG_WAIT_KLLABLE_RECV = 1 << 5;
    }
}

bitflags! {
    /// Valid actions for seccomp syscall.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.15/source/include/uapi/linux/seccomp.h#L38>.
    struct SeccompAction: u32 {
        const SECCOMP_RET_KILL_PROCESS = 0x80000000; // kill the process
        const SECCOMP_RET_KILL_THREAD  = 0x00000000; // kill the thread
        const SECCOMP_RET_TRAP         = 0x00030000; // disallow and force a SIGSYS
        const SECCOMP_RET_ERRNO        = 0x00050000; // returns an errno
        const SECCOMP_RET_USER_NOTIF   = 0x7fc00000; // notifies userspace
        const SECCOMP_RET_TRACE        = 0x7ff00000; // pass to a tracer or disallow
        const SECCOMP_RET_LOG          = 0x7ffc0000; // allow after logging
        const SECCOMP_RET_ALLOW        = 0x7fff0000; // allow
    }
}
