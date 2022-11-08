use crate::prelude::*;

use crate::{process::Process, syscall::SYS_GETPID};

use super::SyscallReturn;

pub fn sys_getpid() -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_GETPID]", SYS_GETPID);
    let pid = Process::current().pid();
    info!("[sys_getpid]: pid = {}", pid);
    Ok(SyscallReturn::Return(pid as _))
}
