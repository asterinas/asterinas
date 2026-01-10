// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::{FileDesc, get_file_fast},
    prelude::*,
};

pub fn sys_sync(ctx: &Context) -> Result<SyscallReturn> {
    let current_ns_proxy = ctx.thread_local.borrow_ns_proxy();
    let current_mnt_ns = current_ns_proxy.unwrap().mnt_ns();
    current_mnt_ns.sync()?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_syncfs(fd: FileDesc, ctx: &Context) -> Result<SyscallReturn> {
    debug!("fd = {}", fd);

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);
    file.path().fs().sync()?;
    Ok(SyscallReturn::Return(0))
}
