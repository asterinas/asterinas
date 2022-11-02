use crate::prelude::*;

use crate::{process::Process, syscall::SYS_GETPID};

pub fn sys_getpid() -> Result<isize> {
    debug!("[syscall][id={}][SYS_GETPID]", SYS_GETPID);
    let pid = Process::current().pid();
    info!("[sys_getpid]: pid = {}", pid);
    Ok(pid as _)
}
