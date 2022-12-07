use super::SyscallReturn;
use super::SYS_CLOSE;
use crate::log_syscall_entry;
use crate::{fs::file::FileDescripter, prelude::*};

pub fn sys_close(fd: FileDescripter) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_CLOSE);
    debug!("fd = {}", fd);
    let current = current!();
    let mut file_table = current.file_table().lock();
    let _ = file_table.get_file(fd)?;
    file_table.close_file(fd);
    Ok(SyscallReturn::Return(0))
}
