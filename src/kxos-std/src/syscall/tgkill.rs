use crate::prelude::*;

use crate::syscall::SYS_TGKILL;

use super::SyscallReturn;

pub fn sys_tgkill(tgid: u64, pid: u64, signal: u64) -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_TGKILL]", SYS_TGKILL);
    debug!("tgid = {}", tgid);
    debug!("pid = {}", pid);
    warn!("TODO: tgkill do nothing now");
    Ok(SyscallReturn::Return(0))
}
