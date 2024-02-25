// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_SETGID};
use crate::{
    log_syscall_entry,
    prelude::*,
    process::{credentials_mut, Gid},
};

pub fn sys_setgid(gid: i32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SETGID);

    debug!("gid = {}", gid);

    if gid < 0 {
        return_errno_with_message!(Errno::EINVAL, "gid cannot be negative");
    }

    let gid = Gid::new(gid as u32);

    let credentials = credentials_mut();
    credentials.set_gid(gid);

    Ok(SyscallReturn::Return(0))
}
