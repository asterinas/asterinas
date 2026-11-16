// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, sched_getattr::read_linux_sched_attr_from_user};
use crate::{
    prelude::*,
    process::{
        UserNamespace, credentials::capabilities::CapSet, pid_table, posix_thread::AsPosixThread,
    },
    sched::{SchedAttr, SchedPolicy},
    security::lsm::hooks as lsm_hooks,
    thread::Tid,
};

pub(super) fn sys_sched_setattr(
    tid: Tid,
    addr: Vaddr,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    if addr == 0 {
        return_errno_with_message!(Errno::EINVAL, "invalid user space address");
    }
    if flags != 0 {
        // Linux also has no support for any flags yet.
        return_errno_with_message!(Errno::EINVAL, "invalid flags");
    }

    let attr = read_linux_sched_attr_from_user(addr, ctx)?;
    let policy = SchedPolicy::try_from(attr)?;
    update_sched_attr_with(tid, ctx, |attr| {
        attr.set_policy(policy);
        Ok(())
    })?;

    Ok(SyscallReturn::Return(0))
}

pub(super) fn update_sched_attr_with<T>(
    tid: Tid,
    ctx: &Context,
    f: impl FnOnce(&SchedAttr) -> Result<T>,
) -> Result<T> {
    if tid.cast_signed() < 0 {
        return_errno_with_message!(Errno::EINVAL, "all negative TIDs are not valid");
    }

    if tid == 0 {
        return f(ctx.thread.sched_attr());
    }

    let Some(thread) = pid_table::pid_table_mut().get_thread(tid) else {
        return_errno_with_message!(Errno::ESRCH, "the target thread does not exist");
    };

    let target = thread.as_posix_thread().unwrap();
    let target_creds = target.credentials();
    let caller_uid = ctx.posix_thread.credentials().euid();
    if caller_uid != target_creds.ruid() && caller_uid != target_creds.euid() {
        lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
            UserNamespace::get_init_singleton().as_ref(),
            ctx.posix_thread,
            CapSet::SYS_NICE,
        ))?;
    }

    f(thread.sched_attr())
}
