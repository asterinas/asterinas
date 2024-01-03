// SPDX-License-Identifier: MPL-2.0

use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::{credentials_mut, Gid};

use super::{SyscallReturn, SYS_SETFSGID};

pub fn sys_setfsgid(gid: i32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SETFSGID);
    debug!("gid = {}", gid);

    let fsgid = if gid < 0 {
        None
    } else {
        Some(Gid::new(gid as u32))
    };

    let old_fsgid = {
        let credentials = credentials_mut();
        credentials.set_fsgid(fsgid)?
    };

    Ok(SyscallReturn::Return(old_fsgid.as_u32() as _))
}
