use crate::{prelude::*, syscall::SYS_GETUID};

use super::SyscallReturn;

pub fn sys_getuid() -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_GETUID]", SYS_GETUID);
    // TODO: getuid only return a fake uid now;
    Ok(SyscallReturn::Return(0))
}
