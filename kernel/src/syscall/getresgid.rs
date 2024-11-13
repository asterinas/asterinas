// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_getresgid(
    rgid_ptr: Vaddr,
    egid_ptr: Vaddr,
    sgid_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!("rgid_ptr = 0x{rgid_ptr:x}, egid_ptr = 0x{egid_ptr:x}, sgid_ptr = 0x{sgid_ptr:x}");

    let credentials = ctx.posix_thread.credentials();
    let user_space = ctx.user_space();

    let rgid = credentials.rgid();
    user_space.write_val(rgid_ptr, &rgid)?;

    let egid = credentials.egid();
    user_space.write_val(egid_ptr, &egid)?;

    let sgid = credentials.sgid();
    user_space.write_val(sgid_ptr, &sgid)?;

    Ok(SyscallReturn::Return(0))
}
