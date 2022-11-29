use super::SyscallReturn;
use super::SYS_CLOSE;
use crate::{fs::file::FileDescripter, prelude::*};

pub fn sys_close(fd: FileDescripter) -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_CLOSE]", SYS_CLOSE);
    debug!("fd = {}", fd);
    let current = current!();
    let mut file_table = current.file_table().lock();
    match file_table.get_file(fd) {
        None => return_errno!(Errno::EBADF),
        Some(_) => {
            file_table.close_file(fd);
            Ok(SyscallReturn::Return(0))
        }
    }
}
