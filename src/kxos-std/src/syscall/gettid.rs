use crate::prelude::*;

use crate::{process::Process, syscall::SYS_GETTID};

pub fn sys_gettid() -> Result<isize> {
    debug!("[syscall][id={}][SYS_GETTID]", SYS_GETTID);
    // For single-thread process, tid is equal to pid
    let tid = Process::current().pid();
    Ok(tid as _)
}
