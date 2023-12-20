use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::credentials;

use super::{SyscallReturn, SYS_GETEGID};

pub fn sys_getegid() -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETEGID);

    let egid = {
        let credentials = credentials();
        credentials.egid()
    };

    Ok(SyscallReturn::Return(egid.as_u32() as _))
}
