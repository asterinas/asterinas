// SPDX-License-Identifier: MPL-2.0

use crate::process::ResourceType;
use crate::util::{read_val_from_user, write_val_to_user};
use crate::{log_syscall_entry, prelude::*, process::Pid};

use super::SyscallReturn;
use super::SYS_PRLIMIT64;

pub fn sys_prlimit64(
    pid: Pid,
    resource: u32,
    new_rlim_addr: Vaddr,
    old_rlim_addr: Vaddr,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_PRLIMIT64);
    let resource = ResourceType::try_from(resource)?;
    debug!(
        "pid = {}, resource = {:?}, new_rlim_addr = 0x{:x}, old_rlim_addr = 0x{:x}",
        pid, resource, new_rlim_addr, old_rlim_addr
    );
    let current = current!();
    let mut resource_limits = current.resource_limits().lock();
    if old_rlim_addr != 0 {
        let rlimit = resource_limits.get_rlimit(resource);
        write_val_to_user(old_rlim_addr, rlimit)?;
    }
    if new_rlim_addr != 0 {
        let new_rlimit = read_val_from_user(new_rlim_addr)?;
        *resource_limits.get_rlimit_mut(resource) = new_rlimit;
    }
    Ok(SyscallReturn::Return(0))
}
