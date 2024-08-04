// SPDX-License-Identifier: MPL-2.0

use super::{CurrentInfo, SyscallReturn};
use crate::prelude::*;

pub fn sys_getuid(current: CurrentInfo) -> Result<SyscallReturn> {
    let uid = current.posix_thread.credentials().ruid();

    Ok(SyscallReturn::Return(uid.as_u32() as _))
}
