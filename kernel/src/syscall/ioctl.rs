// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file::{
        FileLike, StatusFlags,
        file_table::{FdFlags, FileDesc, WithFileTable, get_file_fast},
    },
    prelude::*,
    process::posix_thread::FileTableRefMut,
    util::ioctl::{RawIoctl, dispatch_ioctl},
};

pub fn sys_ioctl(fd: FileDesc, cmd: u32, arg: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let raw_ioctl = RawIoctl::new(cmd, arg);
    debug!("fd = {}, raw_ioctl = {:#x?}", fd, raw_ioctl,);

    let mut file_table = ctx.thread_local.borrow_file_table_mut();

    // First, handle the ioctl command that affects the file descriptor.
    if let Some(res) = handle_fd_ioctl(&mut file_table, fd, raw_ioctl) {
        res?;
        return Ok(SyscallReturn::Return(0));
    }

    let file = get_file_fast!(&mut file_table, fd);

    // Then, handle the ioctl command the affects the file description.
    let res = if let Some(res) = handle_file_ioctl(&**file, raw_ioctl) {
        res?;
        0
    } else {
        let file_owned = file.into_owned();
        // We have to drop `file_table` because some I/O command will modify the file table
        // (e.g., TIOCGPTPEER).
        drop(file_table);
        file_owned.ioctl(raw_ioctl)?
    };

    Ok(SyscallReturn::Return(res as _))
}

mod ioctl_defs {
    use crate::util::ioctl::{InData, NoData, ioc};

    // Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/asm-generic/ioctls.h>

    pub(super) type SetNonBlocking    = ioc!(FIONBIO,  0x5421, InData<i32>);
    pub(super) type SetAsync          = ioc!(FIOASYNC, 0x5452, InData<i32>);

    pub(super) type SetNotCloseOnExec = ioc!(FIONCLEX, 0x5450, NoData);
    pub(super) type SetCloseOnExec    = ioc!(FIOCLEX,  0x5451, NoData);
}

fn handle_fd_ioctl(
    file_table: &mut FileTableRefMut,
    fd: FileDesc,
    raw_ioctl: RawIoctl,
) -> Option<Result<()>> {
    use ioctl_defs::*;

    dispatch_ioctl!(match raw_ioctl {
        SetNotCloseOnExec => {
            // Clears the close-on-exec flag of the file.
            // Follow the implementation of `fcntl()`.

            Some(file_table.read_with(|inner| {
                let entry = inner.get_entry(fd)?;
                // FIXME: This is racy.
                entry.set_flags(entry.flags() - FdFlags::CLOEXEC);
                Ok(())
            }))
        }
        SetCloseOnExec => {
            // Sets the close-on-exec flag of the file.
            // Follow the implementation of `fcntl()`.

            Some(file_table.read_with(|inner| {
                let entry = inner.get_entry(fd)?;
                // FIXME: This is racy.
                entry.set_flags(entry.flags() | FdFlags::CLOEXEC);
                Ok(())
            }))
        }
        _ => None,
    })
}

fn handle_file_ioctl(file: &dyn FileLike, raw_ioctl: RawIoctl) -> Option<Result<()>> {
    use ioctl_defs::*;

    dispatch_ioctl!(match raw_ioctl {
        cmd @ SetNonBlocking => {
            let handler = || {
                let is_nonblocking = cmd.read()? != 0;

                let mut flags = file.status_flags();
                flags.set(StatusFlags::O_NONBLOCK, is_nonblocking);
                // FIXME: This is racy.
                file.set_status_flags(flags)
            };
            Some(handler())
        }
        cmd @ SetAsync => {
            let handler = || {
                let is_async = cmd.read()? != 0;

                // Setting the `O_ASYNC` flag will cause the kernel to send the owner process a
                // `SIGIO` signal when input/output is possible. The user should first call
                // `fcntl(fd, F_SETOWN, pid)` to specify the process to be notified.
                let mut flags = file.status_flags();
                flags.set(StatusFlags::O_ASYNC, is_async);
                // FIXME: This is racy.
                file.set_status_flags(flags)
            };
            Some(handler())
        }
        _ => None,
    })
}
