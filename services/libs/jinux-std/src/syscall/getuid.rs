use crate::{log_syscall_entry, prelude::*, syscall::SYS_GETUID};

use super::SyscallReturn;

pub fn sys_getuid() -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETUID);
    // TODO: getuid only return a fake uid now;
    Ok(SyscallReturn::Return(0))
}
