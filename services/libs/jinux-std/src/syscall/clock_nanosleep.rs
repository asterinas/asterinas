use core::time::Duration;

use super::SyscallReturn;
use super::SYS_CLOCK_NANOSLEEP;
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::signal::Pauser;
use crate::time::{clockid_t, now_as_duration, timespec_t, ClockID, TIMER_ABSTIME};
use crate::util::{read_val_from_user, write_val_to_user};

pub fn sys_clock_nanosleep(
    clockid: clockid_t,
    flags: i32,
    request_timespec_addr: Vaddr,
    remain_timespec_addr: Vaddr,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_CLOCK_NANOSLEEP);
    let clock_id = ClockID::try_from(clockid)?;
    let abs_time = if flags == 0 {
        false
    } else if flags == TIMER_ABSTIME {
        true
    } else {
        unreachable!()
    };

    let duration = {
        let timespec = read_val_from_user::<timespec_t>(request_timespec_addr)?;
        if abs_time {
            todo!("deal with abs time");
        }
        Duration::from(timespec)
    };

    debug!(
        "clockid = {:?}, abs_time = {}, duration = {:?}, remain_timespec_addr = 0x{:x}",
        clock_id, abs_time, duration, remain_timespec_addr
    );

    let start_time = now_as_duration(&clock_id)?;

    // FIXME: sleeping thread can only be interrupted by signals that will call signal handler or terminate
    // current process. i.e., the signals that should be ignored will not interrupt sleeping thread.
    let pauser = Pauser::new();

    let res = pauser.pause_until_or_timeout(|| None, &duration);
    match res {
        Err(e) if e.error() == Errno::ETIME => Ok(SyscallReturn::Return(0)),
        Err(e) if e.error() == Errno::EINTR => {
            let end_time = now_as_duration(&clock_id)?;

            if end_time >= start_time + duration {
                return Ok(SyscallReturn::Return(0));
            }

            if remain_timespec_addr != 0 {
                let remaining_duration = (start_time + duration) - end_time;
                let remaining_timespec = timespec_t::from(remaining_duration);
                write_val_to_user(remain_timespec_addr, &remaining_timespec)?;
            }

            return_errno_with_message!(Errno::EINTR, "sleep was interrupted");
        }
        Ok(()) | Err(_) => unreachable!(),
    }
}
