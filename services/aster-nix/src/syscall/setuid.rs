// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_SETUID};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::{credentials_mut, Uid};

pub fn sys_setuid(uid: i32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SETUID);

    debug!("uid = {}", uid);

    if uid < 0 {
        return_errno_with_message!(Errno::EINVAL, "uid cannot be negative");
    }

    let uid = Uid::new(uid as u32);

    let credentials = credentials_mut();
    credentials.set_uid(uid);

    Ok(SyscallReturn::Return(0))
}
