// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::{FdFlags, FileDesc},
        utils::StatusFlags,
    },
    prelude::*,
    process::Pid,
};

pub fn sys_fcntl(fd: FileDesc, cmd: i32, arg: u64, ctx: &Context) -> Result<SyscallReturn> {
    let fcntl_cmd = FcntlCmd::try_from(cmd)?;
    debug!("fd = {}, cmd = {:?}, arg = {}", fd, fcntl_cmd, arg);
    let current = ctx.process;
    match fcntl_cmd {
        FcntlCmd::F_DUPFD => {
            let mut file_table = current.file_table().lock();
            let new_fd = file_table.dup(fd, arg as FileDesc, FdFlags::empty())?;
            Ok(SyscallReturn::Return(new_fd as _))
        }
        FcntlCmd::F_DUPFD_CLOEXEC => {
            let mut file_table = current.file_table().lock();
            let new_fd = file_table.dup(fd, arg as FileDesc, FdFlags::CLOEXEC)?;
            Ok(SyscallReturn::Return(new_fd as _))
        }
        FcntlCmd::F_GETFD => {
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
            let file_table = current.file_table().lock();
            let entry = file_table.get_entry(fd)?;
            entry.set_flags(flags);
            Ok(SyscallReturn::Return(0))
        }
        FcntlCmd::F_GETFL => {
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
        FcntlCmd::F_SETOWN => {
            let file_table = current.file_table().lock();
            let file_entry = file_table.get_entry(fd)?;
            // A process ID is specified as a positive value; a process group ID is specified as a negative value.
            let abs_arg = (arg as i32).unsigned_abs();
            if abs_arg > i32::MAX as u32 {
                return_errno_with_message!(Errno::EINVAL, "process (group) id overflowed");
            }
            let pid = Pid::try_from(abs_arg)
                .map_err(|_| Error::with_message(Errno::EINVAL, "invalid process (group) id"))?;
            file_entry.set_owner(pid)?;
            Ok(SyscallReturn::Return(0))
        }
        FcntlCmd::F_GETOWN => {
            let file_table = current.file_table().lock();
            let file_entry = file_table.get_entry(fd)?;
            let pid = file_entry.owner().unwrap_or(0);
            Ok(SyscallReturn::Return(pid as _))
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
    F_SETOWN = 8,
    F_GETOWN = 9,
    F_DUPFD_CLOEXEC = 1030,
}
