// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_getgroups(size: i32, group_list_addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    debug!("size = {}, group_list_addr = 0x{:x}", size, group_list_addr);

    if size < 0 {
        return_errno_with_message!(Errno::EINVAL, "size cannot be negative");
    }

    let credentials = ctx.posix_thread.credentials();
    let groups = credentials.groups();

    if size == 0 {
        return Ok(SyscallReturn::Return(groups.len() as _));
    }

    if groups.len() > size as usize {
        return_errno_with_message!(
            Errno::EINVAL,
            "size is less than the number of supplementary group IDs"
        );
    }

    let user_space = ctx.user_space();
    for (idx, gid) in groups.iter().enumerate() {
        let addr = group_list_addr + idx * core::mem::size_of_val(gid);
        user_space.write_val(addr, gid)?;
    }

    Ok(SyscallReturn::Return(groups.len() as _))
}
