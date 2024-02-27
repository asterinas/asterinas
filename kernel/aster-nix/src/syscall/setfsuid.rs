// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_SETFSUID};
use crate::{
    log_syscall_entry,
    prelude::*,
    process::{credentials_mut, Uid},
};

pub fn sys_setfsuid(uid: i32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SETFSUID);
    debug!("uid = {}", uid);

    let fsuid = if uid < 0 {
        None
    } else {
        Some(Uid::new(uid as u32))
    };

    let old_fsuid = {
        let credentials = credentials_mut();
        credentials.set_fsuid(fsuid)?
    };

    Ok(SyscallReturn::Return(old_fsuid.as_u32() as _))
}
