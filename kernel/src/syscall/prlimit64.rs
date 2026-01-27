// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{
        Pid, Process, ResourceType,
        credentials::capabilities::CapSet,
        posix_thread::AsPosixThread,
        process_table,
        rlimit::{RawRLimit64, SYSCTL_NR_OPEN},
    },
};

pub fn sys_getrlimit(resource: u32, rlim_addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let old_raw = do_prlimit64(&ctx.process, resource, None, ctx)?;
    ctx.user_space().write_val(rlim_addr, &old_raw)?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_setrlimit(resource: u32, new_rlim_addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let new_raw = ctx.user_space().read_val(new_rlim_addr)?;
    do_prlimit64(&ctx.process, resource, Some(new_raw), ctx)?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_prlimit64(
    pid: Pid,
    resource: u32,
    new_rlim_addr: Vaddr,
    old_rlim_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();

    let new_raw = if new_rlim_addr == 0 {
        None
    } else {
        Some(user_space.read_val(new_rlim_addr)?)
    };

    let old_raw = if pid == 0 || pid == ctx.process.pid() {
        do_prlimit64(&ctx.process, resource, new_raw, ctx)?
    } else {
        let target_process = process_table::get_process(pid).ok_or_else(|| {
            Error::with_message(Errno::ESRCH, "the target process does not exist")
        })?;
        // Check permissions
        check_rlimit_perm(&target_process, ctx)?;
        do_prlimit64(&target_process, resource, new_raw, ctx)?
    };

    if old_rlim_addr != 0 {
        user_space.write_val(old_rlim_addr, &old_raw)?;
    }

    Ok(SyscallReturn::Return(0))
}

fn do_prlimit64(
    target_process: &Process,
    resource: u32,
    new_raw: Option<RawRLimit64>,
    ctx: &Context,
) -> Result<RawRLimit64> {
    let resource = ResourceType::try_from(resource)?;
    debug!(
        "pid = {}, resource = {:?}, new_raw = {:?}",
        target_process.pid(),
        resource,
        new_raw,
    );

    let rlimit = {
        let resource_limits = target_process.resource_limits();
        resource_limits.get_rlimit(resource)
    };

    let old_raw = if let Some(new_raw) = new_raw {
        if resource == ResourceType::RLIMIT_NOFILE && new_raw.max > SYSCTL_NR_OPEN {
            return_errno_with_message!(Errno::EPERM, "the new limit exceeds the system limit");
        }
        rlimit.set_raw_rlimit(new_raw, ctx)?
    } else {
        rlimit.get_raw_rlimit()
    };

    Ok(old_raw)
}

/// Checks whether the current process has permission to access
/// the resource limits of the target process.
// Reference: <https://man7.org/linux/man-pages/man2/prlimit.2.html>
fn check_rlimit_perm(target_process: &Process, ctx: &Context) -> Result<()> {
    if target_process
        .user_ns()
        .lock()
        .check_cap(CapSet::SYS_RESOURCE, ctx.posix_thread)
        .is_ok()
    {
        return Ok(());
    }

    let (current_ruid, current_rgid) = {
        let current_cred = ctx.posix_thread.credentials();
        (current_cred.ruid(), current_cred.rgid())
    };

    let target_cred = target_process
        .main_thread()
        .as_posix_thread()
        .unwrap()
        .credentials();

    if current_ruid == target_cred.ruid()
        && current_ruid == target_cred.euid()
        && current_ruid == target_cred.suid()
        && current_rgid == target_cred.rgid()
        && current_rgid == target_cred.egid()
        && current_rgid == target_cred.sgid()
    {
        return Ok(());
    }

    return_errno_with_message!(
        Errno::EPERM,
        "accessing the resource limits of the target process is not allowed"
    )
}
