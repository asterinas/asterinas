// SPDX-License-Identifier: MPL-2.0

use crate::{log_syscall_entry, prelude::*};

use crate::syscall::SYS_GETTID;

use super::SyscallReturn;

pub fn sys_gettid() -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETTID);
    let current_thread = current_thread!();
    let tid = current_thread.tid();
    Ok(SyscallReturn::Return(tid as _))
}
