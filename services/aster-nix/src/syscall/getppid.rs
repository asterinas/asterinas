// SPDX-License-Identifier: MPL-2.0

use crate::log_syscall_entry;
use crate::prelude::*;

use super::SyscallReturn;
use super::SYS_GETPPID;

pub fn sys_getppid() -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETPPID);
    let current = current!();
    let parent = current.parent();
    match parent {
        None => Ok(SyscallReturn::Return(0)),
        Some(parent) => Ok(SyscallReturn::Return(parent.pid() as _)),
    }
}
