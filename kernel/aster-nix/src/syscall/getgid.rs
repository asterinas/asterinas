// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{prelude::*, process::credentials};

pub fn sys_getgid() -> Result<SyscallReturn> {
    let gid = {
        let credentials = credentials();
        credentials.rgid()
    };

    Ok(SyscallReturn::Return(gid.as_u32() as _))
}
