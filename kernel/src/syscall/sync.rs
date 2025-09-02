// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_sync(ctx: &Context) -> Result<SyscallReturn> {
    let current_ns_proxy = ctx.thread_local.borrow_ns_proxy();
    let current_mnt_ns = current_ns_proxy.unwrap().mnt_ns();
    current_mnt_ns.sync()?;
    Ok(SyscallReturn::Return(0))
}
