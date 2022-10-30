use crate::prelude::*;

use crate::{process::Process, syscall::SYS_GETPID};

use super::SyscallResult;

pub fn sys_getpid() -> SyscallResult {
    debug!("[syscall][id={}][SYS_GETPID]", SYS_GETPID);
    let pid = Process::current().pid();
    info!("[sys_getpid]: pid = {}", pid);
    SyscallResult::Return(pid as _)
}
