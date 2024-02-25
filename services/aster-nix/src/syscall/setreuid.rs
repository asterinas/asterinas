// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_SETREUID};
use crate::{
    log_syscall_entry,
    prelude::*,
    process::{credentials_mut, Uid},
};

pub fn sys_setreuid(ruid: i32, euid: i32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SETREUID);
    debug!("ruid = {}, euid = {}", ruid, euid);

    let ruid = if ruid > 0 {
        Some(Uid::new(ruid as u32))
    } else {
        None
    };

    let euid = if euid > 0 {
        Some(Uid::new(euid as u32))
    } else {
        None
    };

    let credentials = credentials_mut();
    credentials.set_reuid(ruid, euid)?;

    Ok(SyscallReturn::Return(0))
}
