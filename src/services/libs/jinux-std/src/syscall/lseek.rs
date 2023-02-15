use crate::log_syscall_entry;
use crate::{fs::file_table::FileDescripter, prelude::*};

use super::SyscallReturn;
use super::SYS_LSEEK;

pub fn sys_lseek(fd: FileDescripter, offset: usize, whence: u32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_LSEEK);
    debug!("fd = {}, offset = {}, whence = {}", fd, offset, whence);
    // TODO: do lseek
    Ok(SyscallReturn::Return(0))
}
