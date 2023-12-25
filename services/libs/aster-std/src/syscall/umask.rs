use super::SyscallReturn;
use super::SYS_UMASK;
use crate::{log_syscall_entry, prelude::*};

pub fn sys_umask(mask: u16) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_UMASK);
    debug!("mask = 0o{:o}", mask);
    let current = current!();
    let old_mask = current.umask().write().set(mask);
    Ok(SyscallReturn::Return(old_mask as _))
}
