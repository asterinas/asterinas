use super::{SyscallReturn, SYS_GETRESGID};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::credentials;
use crate::util::write_val_to_user;

pub fn sys_getresgid(rgid_ptr: Vaddr, egid_ptr: Vaddr, sgid_ptr: Vaddr) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETRESGID);
    debug!("rgid_ptr = 0x{rgid_ptr:x}, egid_ptr = 0x{egid_ptr:x}, sgid_ptr = 0x{sgid_ptr:x}");

    let credentials = credentials();

    let rgid = credentials.rgid();
    write_val_to_user(rgid_ptr, &rgid)?;

    let egid = credentials.egid();
    write_val_to_user(egid_ptr, &egid)?;

    let sgid = credentials.sgid();
    write_val_to_user(sgid_ptr, &sgid)?;

    Ok(SyscallReturn::Return(0))
}
