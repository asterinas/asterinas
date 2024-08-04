// SPDX-License-Identifier: MPL-2.0

use super::{CurrentInfo, SyscallReturn};
use crate::prelude::*;

pub fn sys_getppid(current: CurrentInfo) -> Result<SyscallReturn> {
    let parent = current.process.parent();
    match parent {
        None => Ok(SyscallReturn::Return(0)),
        Some(parent) => Ok(SyscallReturn::Return(parent.pid() as _)),
    }
}
