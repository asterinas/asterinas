// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{file_table::FileDesc, inode_handle::InodeHandle},
    prelude::*,
};

pub fn sys_fsync(fd: FileDesc, ctx: &Context) -> Result<SyscallReturn> {
    debug!("fd = {}", fd);

    let dentry = ctx.process.file_table().lock_with(|file_table| {
        let file = file_table.get_file(fd)?;
        let inode_handle = file
            .downcast_ref::<InodeHandle>()
            .ok_or(Error::with_message(Errno::EINVAL, "not inode"))?;
        Result::Ok(inode_handle.dentry().clone())
    })?;

    dentry.sync_all()?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_fdatasync(fd: FileDesc, ctx: &Context) -> Result<SyscallReturn> {
    debug!("fd = {}", fd);

    let dentry = ctx.process.file_table().lock_with(|file_table| {
        let file = file_table.get_file(fd)?;
        let inode_handle = file
            .downcast_ref::<InodeHandle>()
            .ok_or(Error::with_message(Errno::EINVAL, "not inode"))?;
        Result::Ok(inode_handle.dentry().clone())
    })?;

    dentry.sync_data()?;
    Ok(SyscallReturn::Return(0))
}
