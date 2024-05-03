// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{prelude::*, process::credentials};

pub fn sys_getuid() -> Result<SyscallReturn> {
    let uid = {
        let credentials = credentials();
        credentials.ruid()
    };

    Ok(SyscallReturn::Return(uid.as_u32() as _))
}
