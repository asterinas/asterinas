use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::{credentials_mut, Uid};

use super::{SyscallReturn, SYS_SETRESUID};

pub fn sys_setresuid(ruid: i32, euid: i32, suid: i32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SETRESUID);

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

    let suid = if suid > 0 {
        Some(Uid::new(suid as u32))
    } else {
        None
    };

    debug!("ruid = {:?}, euid = {:?}, suid = {:?}", ruid, euid, suid);

    let credentials = credentials_mut();

    credentials.set_resuid(ruid, euid, suid)?;

    Ok(SyscallReturn::Return(0))
}
