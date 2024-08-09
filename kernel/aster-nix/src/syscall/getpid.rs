// SPDX-License-Identifier: MPL-2.0

use super::{CurrentInfo, SyscallReturn};
use crate::prelude::*;

pub fn sys_getpid(current: CurrentInfo) -> Result<SyscallReturn> {
    let pid = current.process.pid();
    debug!("[sys_getpid]: pid = {}", pid);
    Ok(SyscallReturn::Return(pid as _))
}
