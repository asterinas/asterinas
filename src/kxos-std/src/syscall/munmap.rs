use crate::prelude::*;

use super::SyscallReturn;
use super::SYS_MUNMAP;

pub fn sys_munmap(addr: Vaddr, len: usize) -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_READ]", SYS_MUNMAP);
    debug!("addr = 0x{:x}, len = {}", addr, len);
    //TODO: do munmap
    Ok(SyscallReturn::Return(0))
}
