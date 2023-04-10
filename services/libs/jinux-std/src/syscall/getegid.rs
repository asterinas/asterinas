use crate::{log_syscall_entry, prelude::*, syscall::SYS_GETEGID};

use super::SyscallReturn;

pub fn sys_getegid() -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETEGID);
    // TODO: getegid only return a fake egid now
    Ok(SyscallReturn::Return(0))
}
