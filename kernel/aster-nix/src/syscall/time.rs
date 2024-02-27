// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_TIME};
use crate::{log_syscall_entry, prelude::*, time::SystemTime, util::write_val_to_user};

pub fn sys_time(tloc: Vaddr) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_TIME);
    debug!("tloc = 0x{tloc:x}");
    let now = SystemTime::now();
    let now_as_secs = now.duration_since(&SystemTime::UNIX_EPOCH)?.as_secs();
    write_val_to_user(tloc, &now_as_secs)?;
    Ok(SyscallReturn::Return(0))
}
