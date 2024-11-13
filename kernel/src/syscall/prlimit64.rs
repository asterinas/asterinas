// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{Pid, ResourceType},
};

pub fn sys_getrlimit(resource: u32, rlim_addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let resource = ResourceType::try_from(resource)?;
    debug!("resource = {:?}, rlim_addr = 0x{:x}", resource, rlim_addr);
    let resource_limits = ctx.process.resource_limits().lock();
    let rlimit = resource_limits.get_rlimit(resource);
    ctx.user_space().write_val(rlim_addr, rlimit)?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_setrlimit(resource: u32, new_rlim_addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let resource = ResourceType::try_from(resource)?;
    debug!(
        "resource = {:?}, new_rlim_addr = 0x{:x}",
        resource, new_rlim_addr
    );
    let new_rlimit = ctx.user_space().read_val(new_rlim_addr)?;
    let mut resource_limits = ctx.process.resource_limits().lock();
    *resource_limits.get_rlimit_mut(resource) = new_rlimit;
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
    let mut resource_limits = ctx.process.resource_limits().lock();
    if old_rlim_addr != 0 {
        let rlimit = resource_limits.get_rlimit(resource);
        ctx.user_space().write_val(old_rlim_addr, rlimit)?;
    }
    if new_rlim_addr != 0 {
        let new_rlimit = ctx.user_space().read_val(new_rlim_addr)?;
        *resource_limits.get_rlimit_mut(resource) = new_rlimit;
    }
    Ok(SyscallReturn::Return(0))
}
