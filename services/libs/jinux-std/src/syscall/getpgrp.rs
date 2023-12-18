use super::{SyscallReturn, SYS_GETPGRP};
use crate::{log_syscall_entry, prelude::*};

pub fn sys_getpgrp() -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETPGRP);
    let current = current!();
    Ok(SyscallReturn::Return(current.pgid() as _))
}
