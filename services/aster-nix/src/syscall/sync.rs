// SPDX-License-Identifier: MPL-2.0

use crate::log_syscall_entry;
use crate::prelude::*;

use super::SyscallReturn;
use super::SYS_SYNC;

pub fn sys_sync() -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SYNC);

    crate::fs::rootfs::root_mount().sync()?;
    Ok(SyscallReturn::Return(0))
}
