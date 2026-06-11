// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{Gid, credentials::capabilities::CapSet, posix_thread::ContextPthreadAdminApi},
    security::lsm::hooks as lsm_hooks,
};

pub fn sys_setgroups(size: usize, group_list_addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    debug!("size = {}, group_list_addr = 0x{:x}", size, group_list_addr);

    lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
        ctx.thread_local.borrow_user_ns().as_ref(),
        ctx.posix_thread,
        CapSet::SETGID,
    ))?;

    if size > NGROUPS_MAX {
        return_errno_with_message!(Errno::EINVAL, "size cannot be greater than NGROUPS_MAX");
    }

    let mut new_groups = BTreeSet::new();
    for idx in 0..size {
        let addr = group_list_addr + idx * size_of::<Gid>();
        let gid = ctx.user_space().read_val(addr)?;
        new_groups.insert(gid);
    }

    let credentials = ctx.credentials_mut();
    *credentials.groups_mut() = new_groups;

    Ok(SyscallReturn::Return(0))
}

const NGROUPS_MAX: usize = 65536;
