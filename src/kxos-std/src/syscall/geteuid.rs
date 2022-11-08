use crate::{prelude::*, syscall::SYS_GETEUID};

use super::SyscallReturn;

pub fn sys_geteuid() -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_GETEUID]", SYS_GETEUID);
    warn!("TODO: geteuid only return a fake euid now");
    Ok(SyscallReturn::Return(0))
}
