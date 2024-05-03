// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{prelude::*, process::credentials};

pub fn sys_getegid() -> Result<SyscallReturn> {
    let egid = {
        let credentials = credentials();
        credentials.egid()
    };

    Ok(SyscallReturn::Return(egid.as_u32() as _))
}
