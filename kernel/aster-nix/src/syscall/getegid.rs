// SPDX-License-Identifier: MPL-2.0

use super::{CurrentInfo, SyscallReturn};
use crate::prelude::*;

pub fn sys_getegid(current: CurrentInfo) -> Result<SyscallReturn> {
    let egid = current.posix_thread.credentials().egid();

    Ok(SyscallReturn::Return(egid.as_u32() as _))
}
