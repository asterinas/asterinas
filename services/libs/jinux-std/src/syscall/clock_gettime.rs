use super::SyscallReturn;
use super::SYS_CLOCK_GETTIME;
use crate::{
    log_syscall_entry,
    prelude::*,
    time::{clockid_t, timespec_t, ClockID, SystemTime},
    util::write_val_to_user,
};

pub fn sys_clock_gettime(clockid: clockid_t, timespec_addr: Vaddr) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_CLOCK_GETTIME);
    let clock_id = ClockID::try_from(clockid)?;
    debug!("clockid = {:?}", clock_id);

    let now = SystemTime::now();
    let time_duration = match clock_id {
        ClockID::CLOCK_REALTIME | ClockID::CLOCK_REALTIME_COARSE => {
            now.duration_since(&SystemTime::UNIX_EPOCH)?
        }
        // TODO: Respect other type of clock_id
        _ => {
            warn!(
                "unsupported clock_id: {:?}, treat it as CLOCK_REALTIME",
                clock_id
            );
            now.duration_since(&SystemTime::UNIX_EPOCH)?
        }
    };

    let timespec = timespec_t::from(time_duration);
    write_val_to_user(timespec_addr, &timespec)?;

    Ok(SyscallReturn::Return(0))
}
