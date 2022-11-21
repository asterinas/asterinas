use crate::fs::file::FileDescripter;
use crate::fs::ioctl::IoctlCmd;
use crate::prelude::*;

use super::SyscallReturn;
use super::SYS_IOCTL;

pub fn sys_ioctl(fd: FileDescripter, cmd: u32, arg: Vaddr) -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_IOCTL]", SYS_IOCTL);
    let ioctl_cmd = IoctlCmd::try_from(cmd)?;
    debug!(
        "fd = {}, ioctl_cmd = {:?}, arg = 0x{:x}",
        fd, ioctl_cmd, arg
    );
    let current = current!();
    let file_table = current.file_table().lock();
    match file_table.get_file(fd) {
        None => return_errno_with_message!(Errno::EBADF, "Fd does not exist"),
        Some(file) => {
            let res = file.ioctl(ioctl_cmd, arg)?;
            return Ok(SyscallReturn::Return(res as _));
        }
    }
}
