// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::{get_file_fast, FileDesc},
    prelude::*,
};

pub fn sys_fsync(fd: FileDesc, ctx: &Context) -> Result<SyscallReturn> {
    debug!("fd = {}", fd);

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);
    let path = file.as_inode_or_err()?.path();
    path.sync_all()?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_fdatasync(fd: FileDesc, ctx: &Context) -> Result<SyscallReturn> {
    debug!("fd = {}", fd);

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);
    let path = file.as_inode_or_err()?.path();
    path.sync_data()?;
    Ok(SyscallReturn::Return(0))
}
