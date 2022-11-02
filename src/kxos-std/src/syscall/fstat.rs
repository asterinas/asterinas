use kxos_frame::vm::VmIo;

use crate::fs::stat::Stat;
use crate::prelude::*;

use crate::syscall::SYS_FSTAT;

pub fn sys_fstat(fd: u64, stat_buf_ptr: Vaddr) -> Result<isize> {
    debug!("[syscall][id={}][SYS_FSTAT]", SYS_FSTAT);
    debug!("fd = {}", fd);
    debug!("stat_buf_addr = 0x{:x}", stat_buf_ptr);

    let current = current!();
    let vm_space = current
        .vm_space()
        .expect("[Internel Error] User process should have vm space");
    if fd == 1 {
        let stat = Stat::stdout_stat();
        vm_space
            .write_val(stat_buf_ptr, &stat)
            .expect("Write value failed");
        return Ok(0);
    }
    warn!("TODO: fstat only returns fake result now.");
    Ok(0)
}
