// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{rlimit::RLimit64, Pid, ResourceType},
};

pub fn sys_getrlimit(resource: u32, rlim_addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let resource = ResourceType::try_from(resource)?;
    debug!("resource = {:?}, rlim_addr = 0x{:x}", resource, rlim_addr);
    let resource_limits = ctx.process.resource_limits();
    let rlimit = resource_limits.get_rlimit(resource);
    let rlimit_pod = rlimit.get_pod();
    ctx.user_space().write_val(rlim_addr, &rlimit_pod)?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_setrlimit(resource: u32, new_rlim_addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let resource = ResourceType::try_from(resource)?;
    debug!(
        "resource = {:?}, new_rlim_addr = 0x{:x}",
        resource, new_rlim_addr
    );
    let new_rlimit_pod: [u64; 2] = ctx.user_space().read_val(new_rlim_addr)?;
    if !RLimit64::is_valid_pod(&new_rlimit_pod) {
        return_errno_with_message!(Errno::EINVAL, "invalid rlimit");
    }
    let resource_limits = ctx.process.resource_limits();
    resource_limits
        .get_rlimit(resource)
        .set_from_pod(new_rlimit_pod);
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
    let resource_limits = ctx.process.resource_limits();
    if old_rlim_addr != 0 {
        let rlimit = resource_limits.get_rlimit(resource);
        let rlimit_pod = rlimit.get_pod();
        ctx.user_space().write_val(old_rlim_addr, &rlimit_pod)?;
    }
    if new_rlim_addr != 0 {
        let new_rlimit_pod: [u64; 2] = ctx.user_space().read_val(new_rlim_addr)?;
        debug!("new_rlimit = {:?}", new_rlimit_pod);
        if !RLimit64::is_valid_pod(&new_rlimit_pod) {
            return_errno_with_message!(Errno::EINVAL, "invalid rlimit");
        }
        resource_limits
            .get_rlimit(resource)
            .set_from_pod(new_rlimit_pod);
    }
    Ok(SyscallReturn::Return(0))
}
