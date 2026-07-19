// SPDX-License-Identifier: MPL-2.0

use crate::{net::uts_ns::UtsField, prelude::*, syscall::SyscallReturn};

pub fn sys_setdomainname(addr: Vaddr, len: usize, ctx: &Context) -> Result<SyscallReturn> {
    let new_domain_name = UtsField::read_from(addr, len, ctx)?;

    let ns_proxy_ref = ctx.thread_local.borrow_ns_proxy();
    let ns_proxy = ns_proxy_ref.unwrap();
    ns_proxy
        .uts_ns()
        .set_domainname(new_domain_name, ctx.posix_thread)?;

    Ok(SyscallReturn::Return(0))
}
