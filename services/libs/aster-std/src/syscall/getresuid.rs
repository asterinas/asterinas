use super::{SyscallReturn, SYS_GETRESUID};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::credentials;
use crate::util::write_val_to_user;

pub fn sys_getresuid(ruid_ptr: Vaddr, euid_ptr: Vaddr, suid_ptr: Vaddr) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETRESUID);
    debug!("ruid_ptr = 0x{ruid_ptr:x}, euid_ptr = 0x{euid_ptr:x}, suid_ptr = 0x{suid_ptr:x}");

    let credentials = credentials();

    let ruid = credentials.ruid();
    write_val_to_user(ruid_ptr, &ruid)?;

    let euid = credentials.euid();
    write_val_to_user(euid_ptr, &euid)?;

    let suid = credentials.suid();
    write_val_to_user(suid_ptr, &suid)?;

    Ok(SyscallReturn::Return(0))
}
