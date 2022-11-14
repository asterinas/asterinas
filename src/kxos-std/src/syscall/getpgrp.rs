use super::{SyscallReturn, SYS_GETPGRP};
use crate::prelude::*;

pub fn sys_getpgrp() -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_GETPGRP]", SYS_GETPGRP);
    let current = current!();
    Ok(SyscallReturn::Return(current.pgid() as _))
}
