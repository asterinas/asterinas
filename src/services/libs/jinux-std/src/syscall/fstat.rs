use jinux_frame::vm::VmIo;

use crate::fs::utils::Stat;
use crate::{log_syscall_entry, prelude::*};

use crate::syscall::{SyscallReturn, SYS_FSTAT};

pub fn sys_fstat(fd: u64, stat_buf_ptr: Vaddr) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_FSTAT);
    debug!("fd = {}, stat_buf_addr = 0x{:x}", fd, stat_buf_ptr);

    let current = current!();
    let root_vmar = current.root_vmar();
    if fd == 1 {
        let stat = Stat::stdout_stat();
        root_vmar.write_val(stat_buf_ptr, &stat)?;
        return Ok(SyscallReturn::Return(0));
    }
    // TODO: fstat only returns fake result now
    Ok(SyscallReturn::Return(0))
}
