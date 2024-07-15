// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::FileDesc,
        utils::{IoctlCmd, StatusFlags},
    },
    prelude::*,
    util::read_val_from_user,
};

pub fn sys_ioctl(fd: FileDesc, cmd: u32, arg: Vaddr) -> Result<SyscallReturn> {
    let ioctl_cmd = IoctlCmd::try_from(cmd)?;
    debug!(
        "fd = {}, ioctl_cmd = {:?}, arg = 0x{:x}",
        fd, ioctl_cmd, arg
    );
    let current = current!();
    let file_table = current.file_table().lock();
    let file = file_table.get_file(fd)?;
    let res = match ioctl_cmd {
        IoctlCmd::FIONBIO => {
            let is_nonblocking = read_val_from_user::<i32>(arg)? != 0;
            let mut flags = file.status_flags();
            flags.set(StatusFlags::O_NONBLOCK, is_nonblocking);
            file.set_status_flags(flags)?;
            0
        }
        IoctlCmd::FIOASYNC => {
            let is_async = read_val_from_user::<i32>(arg)? != 0;
            let mut flags = file.status_flags();

            // Set `O_ASYNC` flags will send `SIGIO` signal to a process when
            // I/O is possible, user should call `fcntl(fd, F_SETOWN, pid)`
            // first to let the kernel know just whom to notify.
            flags.set(StatusFlags::O_ASYNC, is_async);
            file.set_status_flags(flags)?;
            0
        }
        _ => file.ioctl(ioctl_cmd, arg)?,
    };
    Ok(SyscallReturn::Return(res as _))
}
