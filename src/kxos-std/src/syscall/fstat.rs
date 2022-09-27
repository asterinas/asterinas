use kxos_frame::{debug, warn};

use crate::syscall::{SyscallResult, SYS_FSTAT};

pub fn sys_fstat(fd: u64, stat_buf_addr: u64) -> SyscallResult {
    debug!("[syscall][id={}][SYS_FSTAT]", SYS_FSTAT);
    debug!("fd = {}", fd);
    debug!("stat_buf_addr = 0x{:x}", stat_buf_addr);
    warn!("TODO: fstat only returns fake result now.");
    SyscallResult::Return(0)
}
