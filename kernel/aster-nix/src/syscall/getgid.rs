// SPDX-License-Identifier: MPL-2.0

use super::{CurrentInfo, SyscallReturn};
use crate::prelude::*;

pub fn sys_getgid(current: CurrentInfo) -> Result<SyscallReturn> {
    let gid = current.posix_thread.credentials().rgid();

    Ok(SyscallReturn::Return(gid.as_u32() as _))
}
