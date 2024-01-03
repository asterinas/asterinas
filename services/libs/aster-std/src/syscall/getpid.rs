// SPDX-License-Identifier: MPL-2.0

use crate::{log_syscall_entry, prelude::*};

use crate::syscall::SYS_GETPID;

use super::SyscallReturn;

pub fn sys_getpid() -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETPID);
    let pid = current!().pid();
    debug!("[sys_getpid]: pid = {}", pid);
    Ok(SyscallReturn::Return(pid as _))
}
