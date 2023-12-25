use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::{credentials_mut, Uid};

use super::{SyscallReturn, SYS_SETREUID};

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
