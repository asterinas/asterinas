// SPDX-License-Identifier: MPL-2.0

use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::{credentials_mut, Gid};

use super::{SyscallReturn, SYS_SETRESGID};

pub fn sys_setresgid(rgid: i32, egid: i32, sgid: i32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SETRESGID);

    let rgid = if rgid > 0 {
        Some(Gid::new(rgid as u32))
    } else {
        None
    };

    let egid = if egid > 0 {
        Some(Gid::new(egid as u32))
    } else {
        None
    };

    let sgid = if sgid > 0 {
        Some(Gid::new(sgid as u32))
    } else {
        None
    };

    debug!("rgid = {:?}, egid = {:?}, sgid = {:?}", rgid, egid, sgid);

    let credentials = credentials_mut();
    credentials.set_resgid(rgid, egid, sgid)?;

    Ok(SyscallReturn::Return(0))
}
