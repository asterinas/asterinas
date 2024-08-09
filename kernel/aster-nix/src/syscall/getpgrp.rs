// SPDX-License-Identifier: MPL-2.0

use super::{CurrentInfo, SyscallReturn};
use crate::prelude::*;

pub fn sys_getpgrp(current: CurrentInfo) -> Result<SyscallReturn> {
    Ok(SyscallReturn::Return(current.process.pgid() as _))
}
