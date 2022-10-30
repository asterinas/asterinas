use kxos_frame::vm::VmIo;

use crate::fs::stat::Stat;
use crate::prelude::*;

use crate::syscall::{SyscallResult, SYS_FSTAT};

pub fn sys_fstat(fd: u64, stat_buf_addr: Vaddr) -> SyscallResult {
    debug!("[syscall][id={}][SYS_FSTAT]", SYS_FSTAT);
    debug!("fd = {}", fd);
    debug!("stat_buf_addr = 0x{:x}", stat_buf_addr);

    let current = current!();
    let vm_space = current
        .vm_space()
        .expect("[Internel Error] User process should have vm space");
    if fd == 1 {
        let stat = Stat::stdout_stat();
        vm_space
            .write_val(stat_buf_addr, &stat)
            .expect("Write value failed");
        return SyscallResult::Return(0);
    }
    warn!("TODO: fstat only returns fake result now.");
    SyscallResult::Return(0)
}
