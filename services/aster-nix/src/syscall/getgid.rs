// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_GETGID};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::credentials;

pub fn sys_getgid() -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETGID);

    let gid = {
        let credentials = credentials();
        credentials.rgid()
    };

    Ok(SyscallReturn::Return(gid.as_u32() as _))
}
