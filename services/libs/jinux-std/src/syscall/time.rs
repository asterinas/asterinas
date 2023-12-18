use crate::log_syscall_entry;
use crate::prelude::*;
use crate::time::SystemTime;
use crate::util::write_val_to_user;

use super::SyscallReturn;
use super::SYS_TIME;

pub fn sys_time(tloc: Vaddr) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_TIME);
    debug!("tloc = 0x{tloc:x}");
    let now = SystemTime::now();
    let now_as_secs = now.duration_since(&SystemTime::UNIX_EPOCH)?.as_secs();
    write_val_to_user(tloc, &now_as_secs)?;
    Ok(SyscallReturn::Return(0))
}
