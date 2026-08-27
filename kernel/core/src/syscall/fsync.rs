// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file::{
        SyncMode,
        file_table::{RawFileDesc, get_file_fast},
    },
    prelude::*,
};

pub(super) fn sys_fsync(raw_fd: RawFileDesc, ctx: &Context) -> Result<SyscallReturn> {
    debug!("raw_fd = {}", raw_fd);

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, raw_fd.try_into()?);
    file.sync(SyncMode::Full)?;
    Ok(SyscallReturn::Return(0))
}

pub(super) fn sys_fdatasync(raw_fd: RawFileDesc, ctx: &Context) -> Result<SyscallReturn> {
    debug!("raw_fd = {}", raw_fd);

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, raw_fd.try_into()?);
    file.sync(SyncMode::Data)?;
    Ok(SyscallReturn::Return(0))
}
