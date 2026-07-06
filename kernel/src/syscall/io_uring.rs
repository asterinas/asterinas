// SPDX-License-Identifier: MPL-2.0

//! System call handlers for `io_uring`.

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    fs::file::file_table::FdFlags,
    io_uring::{IoUringContext, IoUringParams, IoUringSetupConfig},
    prelude::*,
};

pub fn sys_io_uring_setup(
    entries: u32,
    params_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let mut params = ctx.user_space().read_val::<IoUringParams>(params_addr)?;
    debug!("entries = {}, params = {:?}", entries, params);

    let setup_config = IoUringSetupConfig::new(entries, &params)?;
    let ring = IoUringContext::new(&setup_config, ctx)?;

    setup_config.write_params(&mut params);
    ctx.user_space().write_val(params_addr, &params)?;

    let file_table = ctx.thread_local.borrow_file_table();
    let mut file_table_locked = file_table.unwrap().write();
    let fd = file_table_locked.insert(ring, FdFlags::CLOEXEC);

    Ok(SyscallReturn::Return(fd.into()))
}
