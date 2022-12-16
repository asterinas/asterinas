use crate::{log_syscall_entry, prelude::*, syscall::SYS_GETGID};

use super::SyscallReturn;

pub fn sys_getgid() -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETGID);
    // TODO: getgid only return a fake gid now"
    Ok(SyscallReturn::Return(0))
}
