use super::SyscallReturn;
use super::SYS_GETTIMEOFDAY;
use crate::{
    log_syscall_entry,
    prelude::*,
    time::{timeval_t, SystemTime},
    util::write_val_to_user,
};

// The use of the timezone structure is obsolete.
// Glibc sets the timezone_addr argument to NULL, so just ignore it.
pub fn sys_gettimeofday(timeval_addr: Vaddr, /* timezone_addr: Vaddr */) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETTIMEOFDAY);
    if timeval_addr == 0 {
        return Ok(SyscallReturn::Return(0));
    }

    let time_val = {
        let now = SystemTime::now();
        let time_duration = now.duration_since(&SystemTime::UNIX_EPOCH)?;
        timeval_t::from(time_duration)
    };
    write_val_to_user(timeval_addr, &time_val)?;

    Ok(SyscallReturn::Return(0))
}
