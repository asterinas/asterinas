// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_SYNC};
use crate::{log_syscall_entry, prelude::*};

pub fn sys_sync() -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SYNC);

    crate::fs::rootfs::root_mount().sync()?;
    Ok(SyscallReturn::Return(0))
}
