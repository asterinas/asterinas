// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::{get_file_fast, FileDesc},
    prelude::*,
};

pub fn sys_fsync(fd: FileDesc, ctx: &Context) -> Result<SyscallReturn> {
    debug!("fd = {}", fd);

    get_file_fast! { let (file_table, file) = fd @ ctx.thread_local };
    let dentry = file.as_inode_or_err()?.dentry();
    dentry.sync_all()?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_fdatasync(fd: FileDesc, ctx: &Context) -> Result<SyscallReturn> {
    debug!("fd = {}", fd);

    get_file_fast! { let (file_table, file) = fd @ ctx.thread_local };
    let dentry = file.as_inode_or_err()?.dentry();
    dentry.sync_data()?;
    Ok(SyscallReturn::Return(0))
}
