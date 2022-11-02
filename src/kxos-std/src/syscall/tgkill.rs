use crate::prelude::*;

use crate::syscall::SYS_TGKILL;

pub fn sys_tgkill(tgid: u64, pid: u64, signal: u64) -> Result<isize> {
    debug!("[syscall][id={}][SYS_TGKILL]", SYS_TGKILL);
    debug!("tgid = {}", tgid);
    debug!("pid = {}", pid);
    warn!("TODO: tgkill do nothing now");
    Ok(0)
}
