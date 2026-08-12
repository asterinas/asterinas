// SPDX-License-Identifier: MPL-2.0

use crate::{
    prelude::*,
    syscall::{SyscallReturn, setdomainname::read_uts_field},
};

pub(super) fn sys_sethostname(addr: Vaddr, len: usize, ctx: &Context) -> Result<SyscallReturn> {
    let new_host_name = read_uts_field(addr, len, ctx)?;

    let ns_proxy_ref = ctx.thread_local.borrow_ns_proxy();
    let ns_proxy = ns_proxy_ref.unwrap();
    ns_proxy
        .uts_ns()
        .set_hostname(new_host_name, ctx.posix_thread)?;

    Ok(SyscallReturn::Return(0))
}
