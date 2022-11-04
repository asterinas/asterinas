use crate::{prelude::*, syscall::SYS_GETGID};

use super::SyscallReturn;

pub fn sys_getgid() -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_GETGID]", SYS_GETGID);
    warn!("TODO: getgid only return a fake gid now");
    Ok(SyscallReturn::Return(0))
}
