// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_GETEUID};
use crate::{log_syscall_entry, prelude::*, process::credentials};

pub fn sys_geteuid() -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETEUID);

    let euid = {
        let credentials = credentials();
        credentials.euid()
    };

    Ok(SyscallReturn::Return(euid.as_u32() as _))
}
