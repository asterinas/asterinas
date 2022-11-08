use crate::{prelude::*, syscall::SYS_GETEGID};

use super::SyscallReturn;

pub fn sys_getegid() -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_GETEGID]", SYS_GETEGID);
    warn!("TODO: getegid only return a fake egid now");
    Ok(SyscallReturn::Return(0))
}
