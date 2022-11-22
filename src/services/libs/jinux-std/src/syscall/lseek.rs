use crate::{fs::file::FileDescripter, prelude::*};

use super::SyscallReturn;
use super::SYS_LSEEK;

pub fn sys_lseek(fd: FileDescripter, offset: usize, whence: u32) -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_LSEEK]", SYS_LSEEK);
    debug!("fd = {}, offset = {}, whence = {}", fd, offset, whence);
    // TODO: do lseek
    Ok(SyscallReturn::Return(0))
}
