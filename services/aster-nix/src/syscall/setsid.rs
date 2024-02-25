// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_SETSID};
use crate::{log_syscall_entry, prelude::*};

pub fn sys_setsid() -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SETSID);

    let current = current!();
    let session = current.to_new_session()?;

    Ok(SyscallReturn::Return(session.sid() as _))
}
