// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::{FdFlags, FileDesc},
    prelude::*,
    process::ResourceType,
};

pub fn sys_dup(old_fd: FileDesc) -> Result<SyscallReturn> {
    debug!("old_fd = {}", old_fd);

    let current = current!();
    let mut file_table = current.file_table().lock();
    let new_fd = file_table.dup(old_fd, 0, FdFlags::empty())?;

    Ok(SyscallReturn::Return(new_fd as _))
}

pub fn sys_dup2(old_fd: FileDesc, new_fd: FileDesc) -> Result<SyscallReturn> {
    debug!("old_fd = {}, new_fd = {}", old_fd, new_fd);

    if old_fd == new_fd {
        let current = current!();
        let file_table = current.file_table().lock();
        let _ = file_table.get_file(old_fd)?;
        return Ok(SyscallReturn::Return(new_fd as _));
    }

    do_dup3(old_fd, new_fd, FdFlags::empty())
}

pub fn sys_dup3(old_fd: FileDesc, new_fd: FileDesc, flags: u32) -> Result<SyscallReturn> {
    debug!("old_fd = {}, new_fd = {}", old_fd, new_fd);

    let fdflag = match flags {
        0x0 => FdFlags::empty(),
        0x80000 => FdFlags::CLOEXEC,
        _ => return_errno_with_message!(Errno::EINVAL, "flags must be O_CLOEXEC or 0"),
    };

    do_dup3(old_fd, new_fd, fdflag)
}

fn do_dup3(old_fd: FileDesc, new_fd: FileDesc, flags: FdFlags) -> Result<SyscallReturn> {
    if old_fd == new_fd {
        return_errno!(Errno::EINVAL);
    }

    let current = current!();
    if new_fd
        >= current
            .resource_limits()
            .lock()
            .get_rlimit(ResourceType::RLIMIT_NOFILE)
            .get_cur() as FileDesc
    {
        return_errno!(Errno::EBADF);
    }

    let mut file_table = current.file_table().lock();
    let _ = file_table.close_file(new_fd);
    let new_fd = file_table.dup(old_fd, new_fd, flags)?;

    Ok(SyscallReturn::Return(new_fd as _))
}
