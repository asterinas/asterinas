use crate::log_syscall_entry;
use crate::prelude::*;

use super::SyscallReturn;
use super::SYS_MUNMAP;

pub fn sys_munmap(addr: Vaddr, len: usize) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_MUNMAP);
    debug!("addr = 0x{:x}, len = {}", addr, len);
    //TODO: do munmap
    Ok(SyscallReturn::Return(0))
}
