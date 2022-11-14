use crate::prelude::*;

use crate::syscall::SYS_GETPID;

use super::SyscallReturn;

pub fn sys_getpid() -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_GETPID]", SYS_GETPID);
    let pid = current!().pid();
    debug!("[sys_getpid]: pid = {}", pid);
    Ok(SyscallReturn::Return(pid as _))
}
