use super::{SyscallReturn, SYS_FCNTL};
use crate::fs::utils::FcntlCmd;
use crate::log_syscall_entry;
use crate::{fs::file_table::FileDescripter, prelude::*};

pub fn sys_fcntl(fd: FileDescripter, cmd: i32, arg: u64) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_FCNTL);
    let fcntl_cmd = FcntlCmd::try_from(cmd)?;
    debug!("fd = {}, cmd = {:?}, arg = {}", fd, fcntl_cmd, arg);
    match fcntl_cmd {
        FcntlCmd::F_DUPFD_CLOEXEC => {
            // FIXME: deal with the cloexec flag
            let current = current!();
            let mut file_table = current.file_table().lock();
            let new_fd = file_table.dup(fd, arg as FileDescripter)?;
            return Ok(SyscallReturn::Return(new_fd as _));
        }
        FcntlCmd::F_SETFD => {
            if arg != 1 {
                panic!("Unknown setfd argument");
            }
            // TODO: Set cloexec
            return Ok(SyscallReturn::Return(0));
        }
        _ => todo!(),
    }
}
