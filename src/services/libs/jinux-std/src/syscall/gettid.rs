use crate::{log_syscall_entry, prelude::*};

use crate::syscall::SYS_GETTID;

use super::SyscallReturn;

pub fn sys_gettid() -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETTID);
    // For single-thread process, tid is equal to pid
    let tid = current!().pid();
    Ok(SyscallReturn::Return(tid as _))
}
