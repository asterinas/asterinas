// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_FCNTL};
use crate::{
    fs::{
        file_table::{FdFlags, FileDescripter},
        utils::StatusFlags,
    },
    log_syscall_entry,
    prelude::*,
};

pub fn sys_fcntl(fd: FileDescripter, cmd: i32, arg: u64) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_FCNTL);
    let fcntl_cmd = FcntlCmd::try_from(cmd)?;
    debug!("fd = {}, cmd = {:?}, arg = {}", fd, fcntl_cmd, arg);
    match fcntl_cmd {
        FcntlCmd::F_DUPFD => {
            let current = current!();
            let mut file_table = current.file_table().lock();
            let new_fd = file_table.dup(fd, arg as FileDescripter, FdFlags::empty())?;
            Ok(SyscallReturn::Return(new_fd as _))
        }
        FcntlCmd::F_DUPFD_CLOEXEC => {
            let current = current!();
            let mut file_table = current.file_table().lock();
            let new_fd = file_table.dup(fd, arg as FileDescripter, FdFlags::CLOEXEC)?;
            Ok(SyscallReturn::Return(new_fd as _))
        }
        FcntlCmd::F_GETFD => {
            let current = current!();
            let file_table = current.file_table().lock();
            let entry = file_table.get_entry(fd)?;
            let fd_flags = entry.flags();
            Ok(SyscallReturn::Return(fd_flags.bits() as _))
        }
        FcntlCmd::F_SETFD => {
            let flags = {
                if arg > u8::MAX.into() {
                    return_errno_with_message!(Errno::EINVAL, "invalid fd flags");
                }
                FdFlags::from_bits(arg as u8)
                    .ok_or(Error::with_message(Errno::EINVAL, "invalid flags"))?
            };
            let current = current!();
            let file_table = current.file_table().lock();
            let entry = file_table.get_entry(fd)?;
            entry.set_flags(flags);
            Ok(SyscallReturn::Return(0))
        }
        FcntlCmd::F_GETFL => {
            let current = current!();
            let file = {
                let file_table = current.file_table().lock();
                file_table.get_file(fd)?.clone()
            };
            let status_flags = file.status_flags();
            let access_mode = file.access_mode();
            Ok(SyscallReturn::Return(
                (status_flags.bits() | access_mode as u32) as _,
            ))
        }
        FcntlCmd::F_SETFL => {
            let current = current!();
            let file = {
                let file_table = current.file_table().lock();
                file_table.get_file(fd)?.clone()
            };
            let new_status_flags = {
                // This cmd can change(set or unset) only the O_APPEND, O_ASYNC, O_DIRECT,
                // O_NOATIME and O_NONBLOCK flags.
                let valid_flags_mask = StatusFlags::O_APPEND
                    | StatusFlags::O_ASYNC
                    | StatusFlags::O_DIRECT
                    | StatusFlags::O_NOATIME
                    | StatusFlags::O_NONBLOCK;
                let mut status_flags = file.status_flags();
                status_flags.remove(valid_flags_mask);
                status_flags.insert(StatusFlags::from_bits_truncate(arg as _) & valid_flags_mask);
                status_flags
            };
            file.set_status_flags(new_status_flags)?;
            Ok(SyscallReturn::Return(0))
        }
    }
}

#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
#[allow(non_camel_case_types)]
enum FcntlCmd {
    F_DUPFD = 0,
    F_GETFD = 1,
    F_SETFD = 2,
    F_GETFL = 3,
    F_SETFL = 4,
    F_DUPFD_CLOEXEC = 1030,
}
