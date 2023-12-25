use crate::log_syscall_entry;
use crate::prelude::*;

use super::{SyscallReturn, SYS_SETSID};

pub fn sys_setsid() -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SETSID);

    let current = current!();
    let session = current.to_new_session()?;

    Ok(SyscallReturn::Return(session.sid() as _))
}
