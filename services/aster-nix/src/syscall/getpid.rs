// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{log_syscall_entry, prelude::*, syscall::SYS_GETPID};

pub fn sys_getpid() -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETPID);
    let pid = current!().pid();
    debug!("[sys_getpid]: pid = {}", pid);
    Ok(SyscallReturn::Return(pid as _))
}
