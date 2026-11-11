// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{
        Process, ResourceType::RLIMIT_NICE, credentials::capabilities::CapSet,
        posix_thread::AsPosixThread,
    },
    sched::Nice,
    security::lsm::hooks as lsm_hooks,
    syscall::get_priority::{PriorityTarget, get_processes},
};

pub(super) fn sys_set_priority(
    which: i32,
    who: u32,
    prio: i32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let prio_target = PriorityTarget::new(which, who, ctx)?;
    let new_nice: Nice = {
        let nice_raw = prio.clamp(
            Nice::MIN.value().get() as i32,
            Nice::MAX.value().get() as i32,
        ) as i8;
        nice_raw.try_into().unwrap()
    };

    debug!(
        "set_priority prio_target: {:?}, new_nice: {:?}",
        prio_target, new_nice
    );

    let processes = get_processes(prio_target)?;
    for process in processes.iter() {
        check_setpriority_permission(process, ctx)?;

        let rlimit = process.resource_limits();
        let limit = (rlimit.get_rlimit(RLIMIT_NICE).get_cur() as i8)
            .try_into()
            .map_err(|msg| Error::with_message(Errno::EINVAL, msg))?;

        if new_nice < limit {
            return_errno!(Errno::EACCES);
        }
        // FIXME: `setpriority` updates only the per-process nice value. Fair
        // scheduler state is kept in each thread's `SchedPolicy::Fair`, so it
        // should be updated from the same source of truth.
        process.nice().store(new_nice, Ordering::Relaxed);
    }

    Ok(SyscallReturn::Return(0))
}

/// Checks whether the caller may change the target process's nice value.
fn check_setpriority_permission(target: &Process, ctx: &Context) -> Result<()> {
    let caller_uid = ctx.posix_thread.credentials().euid();
    let target_credentials = target
        .main_thread()
        .as_posix_thread()
        .unwrap()
        .credentials();

    if caller_uid != target_credentials.ruid() && caller_uid != target_credentials.euid() {
        lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
            target.user_ns().lock().as_ref(),
            ctx.posix_thread,
            CapSet::SYS_NICE,
        ))?;
    }
    Ok(())
}
