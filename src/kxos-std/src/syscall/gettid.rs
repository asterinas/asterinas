use crate::prelude::*;

use crate::{process::Process, syscall::SYS_GETTID};

use super::SyscallReturn;

pub fn sys_gettid() -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_GETTID]", SYS_GETTID);
    // For single-thread process, tid is equal to pid
    let tid = Process::current().pid();
    Ok(SyscallReturn::Return(tid as _))
}
