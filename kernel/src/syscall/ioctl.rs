// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::{FdFlags, FileDesc},
        utils::{IoctlCmd, StatusFlags},
    },
    prelude::*,
};

pub fn sys_ioctl(fd: FileDesc, cmd: u32, arg: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let ioctl_cmd = IoctlCmd::try_from(cmd)?;
    debug!(
        "fd = {}, ioctl_cmd = {:?}, arg = 0x{:x}",
        fd, ioctl_cmd, arg
    );

    let file = {
        let file_table = ctx.process.file_table().lock();
        file_table.get_file(fd)?.clone()
    };
    let res = match ioctl_cmd {
        IoctlCmd::FIONBIO => {
            let is_nonblocking = ctx.user_space().read_val::<i32>(arg)? != 0;
            let mut flags = file.status_flags();
            flags.set(StatusFlags::O_NONBLOCK, is_nonblocking);
            file.set_status_flags(flags)?;
            0
        }
        IoctlCmd::FIOASYNC => {
            let is_async = ctx.user_space().read_val::<i32>(arg)? != 0;
            let mut flags = file.status_flags();

            // Set `O_ASYNC` flags will send `SIGIO` signal to a process when
            // I/O is possible, user should call `fcntl(fd, F_SETOWN, pid)`
            // first to let the kernel know just whom to notify.
            flags.set(StatusFlags::O_ASYNC, is_async);
            file.set_status_flags(flags)?;
            0
        }
        IoctlCmd::FIOCLEX => {
            // Sets the close-on-exec flag of the file.
            // Follow the implementation of fcntl()

            let flags = FdFlags::CLOEXEC;
            let file_table = ctx.process.file_table().lock();
            let entry = file_table.get_entry(fd)?;
            entry.set_flags(flags);
            0
        }
        IoctlCmd::FIONCLEX => {
            // Clears the close-on-exec flag of the file.
            let file_table = ctx.process.file_table().lock();
            let entry = file_table.get_entry(fd)?;
            entry.set_flags(entry.flags() & (!FdFlags::CLOEXEC));
            0
        }
        _ => file.ioctl(ioctl_cmd, arg)?,
    };
    Ok(SyscallReturn::Return(res as _))
}
