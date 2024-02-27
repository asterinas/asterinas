// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_GETEGID};
use crate::{log_syscall_entry, prelude::*, process::credentials};

pub fn sys_getegid() -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETEGID);

    let egid = {
        let credentials = credentials();
        credentials.egid()
    };

    Ok(SyscallReturn::Return(egid.as_u32() as _))
}
