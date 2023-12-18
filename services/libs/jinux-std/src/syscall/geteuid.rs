use crate::{log_syscall_entry, prelude::*, syscall::SYS_GETEUID};

use super::SyscallReturn;

pub fn sys_geteuid() -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETEUID);
    // TODO: geteuid only return a fake euid now"
    Ok(SyscallReturn::Return(0))
}
