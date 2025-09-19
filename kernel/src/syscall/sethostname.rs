// SPDX-License-Identifier: MPL-2.0

use crate::{prelude::*, syscall::SyscallReturn};

pub fn sys_sethostname(addr: Vaddr, len: usize, ctx: &Context) -> Result<SyscallReturn> {
    let ns_proxy_ref = ctx.thread_local.borrow_ns_proxy();
    let ns_proxy = ns_proxy_ref.unwrap();
    ns_proxy.uts_ns().set_hostname(addr, len, ctx)?;
    Ok(SyscallReturn::Return(0))
}
