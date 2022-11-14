use kxos_frame::vm::VmIo;

use crate::fs::stat::Stat;
use crate::prelude::*;

use crate::syscall::{SyscallReturn, SYS_FSTAT};

pub fn sys_fstat(fd: u64, stat_buf_ptr: Vaddr) -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_FSTAT]", SYS_FSTAT);
    debug!("fd = {}, stat_buf_addr = 0x{:x}", fd, stat_buf_ptr);

    let current = current!();
    let vm_space = current.vm_space().unwrap();
    if fd == 1 {
        let stat = Stat::stdout_stat();
        vm_space.write_val(stat_buf_ptr, &stat)?;
        return Ok(SyscallReturn::Return(0));
    }
    // TODO: fstat only returns fake result now
    Ok(SyscallReturn::Return(0))
}
