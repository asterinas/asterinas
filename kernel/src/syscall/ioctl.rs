// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::{get_file_fast, FdFlags, FileDesc, WithFileTable},
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

    let mut file_table = ctx.thread_local.file_table().borrow_mut();
    let file = get_file_fast!(&mut file_table, fd);
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

            file_table.read_with(|inner| {
                let entry = inner.get_entry(fd)?;
                entry.set_flags(entry.flags() | FdFlags::CLOEXEC);
                Ok::<_, Error>(0)
            })?
        }
        IoctlCmd::FIONCLEX => {
            // Clears the close-on-exec flag of the file.
            // Follow the implementation of fcntl()

            file_table.read_with(|inner| {
                let entry = inner.get_entry(fd)?;
                entry.set_flags(entry.flags() - FdFlags::CLOEXEC);
                Ok::<_, Error>(0)
            })?
        }
        // FIXME: ioctl operations involving blocking I/O should be able to restart if interrupted
        _ => {
            let file_owned = file.into_owned();
            // We have to drop `file_table` because some I/O command will modify the file table
            // (e.g., TIOCGPTPEER).
            drop(file_table);

            file_owned.ioctl(ioctl_cmd, arg)?
        }
    };
    Ok(SyscallReturn::Return(res as _))
}
