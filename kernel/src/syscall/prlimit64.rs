// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{
        Pid, Process, ResourceType,
        credentials::capabilities::CapSet,
        posix_thread::{AsPosixThread, PosixThread},
        process_table,
        rlimit::RawRLimit64,
    },
};

pub fn sys_getrlimit(resource: u32, rlim_addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let resource = ResourceType::try_from(resource)?;
    debug!("resource = {:?}, rlim_addr = 0x{:x}", resource, rlim_addr);
    let resource_limits = ctx.process.resource_limits();
    let rlimit = resource_limits.get_rlimit(resource);
    let (cur, max) = rlimit.get_cur_and_max();
    let rlimit_raw = RawRLimit64 { cur, max };
    ctx.user_space().write_val(rlim_addr, &rlimit_raw)?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_setrlimit(resource: u32, new_rlim_addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let resource = ResourceType::try_from(resource)?;
    debug!(
        "resource = {:?}, new_rlim_addr = 0x{:x}",
        resource, new_rlim_addr
    );
    let new_raw: RawRLimit64 = ctx.user_space().read_val(new_rlim_addr)?;
    let resource_limits = ctx.process.resource_limits();
    resource_limits
        .get_rlimit(resource)
        .set_cur_and_max(new_raw.cur, new_raw.max)?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_prlimit64(
    pid: Pid,
    resource: u32,
    new_rlim_addr: Vaddr,
    old_rlim_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let resource = ResourceType::try_from(resource)?;
    debug!(
        "pid = {}, resource = {:?}, new_rlim_addr = 0x{:x}, old_rlim_addr = 0x{:x}",
        pid, resource, new_rlim_addr, old_rlim_addr
    );

    let get_and_set_rlimit = |process: &Process| -> Result<()> {
        let resource_limits = process.resource_limits();

        if old_rlim_addr != 0 {
            let rlimit = resource_limits.get_rlimit(resource);
            let (cur, max) = rlimit.get_cur_and_max();
            let rlimit_raw = RawRLimit64 { cur, max };
            ctx.user_space().write_val(old_rlim_addr, &rlimit_raw)?;
        }
        if new_rlim_addr != 0 {
            let new_raw: RawRLimit64 = ctx.user_space().read_val(new_rlim_addr)?;
            debug!("new_rlimit = {:?}", new_raw);
            resource_limits
                .get_rlimit(resource)
                .set_cur_and_max(new_raw.cur, new_raw.max)?;
        }

        Ok(())
    };

    if pid == 0 || pid == ctx.process.pid() {
        get_and_set_rlimit(ctx.process.as_ref())?;
    } else {
        let target_process = process_table::get_process(pid).ok_or_else(|| {
            Error::with_message(Errno::ESRCH, "the target process does not exist")
        })?;

        // Check permissions
        check_rlimit_perm(target_process.main_thread().as_posix_thread().unwrap(), ctx)?;
        get_and_set_rlimit(target_process.as_ref())?;
    }
    Ok(SyscallReturn::Return(0))
}

/// Checks whether the current process has permission to access
/// the resource limits of the target process.
// Reference: <https://man7.org/linux/man-pages/man2/prlimit.2.html>
fn check_rlimit_perm(target: &PosixThread, ctx: &Context) -> Result<()> {
    let target_process = target.process();

    let current_cred = ctx.posix_thread.credentials();
    let target_cred = target.credentials();

    if target_process
        .user_ns()
        .lock()
        .check_cap(CapSet::SYS_RESOURCE, ctx.posix_thread)
        .is_ok()
    {
        return Ok(());
    }

    let current_ruid = current_cred.ruid();
    let current_rgid = current_cred.rgid();

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
