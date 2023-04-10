use super::SyscallReturn;
use super::SYS_CLOSE;
use crate::log_syscall_entry;
use crate::{fs::file_table::FileDescripter, prelude::*};

pub fn sys_close(fd: FileDescripter) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_CLOSE);
    debug!("fd = {}", fd);
    let current = current!();
    let mut file_table = current.file_table().lock();
    let _ = file_table.get_file(fd)?;
    let file = file_table.close_file(fd).unwrap();
    file.clean_for_close()?;
    Ok(SyscallReturn::Return(0))
}
