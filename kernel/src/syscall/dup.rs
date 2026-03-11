// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file::file_table::{FdFlags, FileDesc, RawFileDesc, get_file_fast},
    prelude::*,
    process::ResourceType,
};

pub fn sys_dup(old_fd: RawFileDesc, ctx: &Context) -> Result<SyscallReturn> {
    debug!("old_fd = {}", old_fd);

    let file_table = ctx.thread_local.borrow_file_table();
    let mut file_table_locked = file_table.unwrap().write();
    let new_fd =
        file_table_locked.dup_ceil(old_fd.try_into()?, FileDesc::ZERO, FdFlags::empty())?;

    Ok(SyscallReturn::Return(new_fd.into()))
}

pub fn sys_dup2(old_fd: RawFileDesc, new_fd: RawFileDesc, ctx: &Context) -> Result<SyscallReturn> {
    debug!("old_fd = {}, new_fd = {}", old_fd, new_fd);

    if old_fd == new_fd {
        let mut file_table = ctx.thread_local.borrow_file_table_mut();
        let _file = get_file_fast!(&mut file_table, old_fd.try_into()?);
        return Ok(SyscallReturn::Return(new_fd as _));
    }

    do_dup3(old_fd, new_fd, FdFlags::empty(), ctx)
}

pub fn sys_dup3(
    old_fd: RawFileDesc,
    new_fd: RawFileDesc,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!("old_fd = {}, new_fd = {}", old_fd, new_fd);

    let fdflag = match flags {
        0x0 => FdFlags::empty(),
        0x80000 => FdFlags::CLOEXEC,
        _ => return_errno_with_message!(Errno::EINVAL, "flags must be O_CLOEXEC or 0"),
    };

    do_dup3(old_fd, new_fd, fdflag, ctx)
}

fn do_dup3(
    old_fd: RawFileDesc,
    new_fd: RawFileDesc,
    flags: FdFlags,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let old_fd = FileDesc::try_from(old_fd)?;
    let new_fd = FileDesc::try_from(new_fd)?;

    if old_fd == new_fd {
        return_errno!(Errno::EINVAL);
    }

    if u64::from(new_fd)
        >= ctx
            .process
            .resource_limits()
            .get_rlimit(ResourceType::RLIMIT_NOFILE)
            .get_cur()
    {
        return_errno!(Errno::EBADF);
    }

    let file_table = ctx.thread_local.borrow_file_table();
    let replaced_file = {
        let mut file_table_locked = file_table.unwrap().write();
        file_table_locked.dup_exact(old_fd, new_fd, flags)?
    };
    drop(replaced_file);

    Ok(SyscallReturn::Return(new_fd.into()))
}
