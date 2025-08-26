// SPDX-License-Identifier: MPL-2.0
use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_uname(old_uname_addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    debug!("old uname addr = 0x{:x}", old_uname_addr);
    let ns_context = ctx.thread_local.borrow_ns_context();
    let uts_name = ns_context.unwrap().uts_ns().uts_name();
    ctx.user_space().write_val(old_uname_addr, uts_name)?;
    Ok(SyscallReturn::Return(0))
}
