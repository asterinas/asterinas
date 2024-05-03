// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_sync() -> Result<SyscallReturn> {
    crate::fs::rootfs::root_mount().sync()?;
    Ok(SyscallReturn::Return(0))
}
