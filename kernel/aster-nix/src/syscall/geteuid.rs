// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{prelude::*, process::credentials};

pub fn sys_geteuid() -> Result<SyscallReturn> {
    let euid = {
        let credentials = credentials();
        credentials.euid()
    };

    Ok(SyscallReturn::Return(euid.as_u32() as _))
}
