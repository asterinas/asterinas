// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_GETUID};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::credentials;

pub fn sys_getuid() -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETUID);

    let uid = {
        let credentials = credentials();
        credentials.ruid()
    };

    Ok(SyscallReturn::Return(uid.as_u32() as _))
}
