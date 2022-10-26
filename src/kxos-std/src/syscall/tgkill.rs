use crate::prelude::*;

use crate::syscall::{SyscallResult, SYS_TGKILL};

pub fn sys_tgkill(tgid: u64, pid: u64, signal: u64) -> SyscallResult {
    debug!("[syscall][id={}][SYS_TGKILL]", SYS_TGKILL);
    debug!("tgid = {}", tgid);
    debug!("pid = {}", pid);
    warn!("TODO: tgkill do nothing now");
    SyscallResult::Return(0)
}
