use core::time::Duration;

use super::SyscallReturn;
use super::SYS_CLOCK_NANOSLEEP;
use crate::{
    log_syscall_entry,
    prelude::*,
    thread::Thread,
    time::{clockid_t, timespec_t, ClockID, TIMER_ABSTIME},
    util::{read_val_from_user, write_val_to_user},
};

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
    let request_timespec = read_val_from_user::<timespec_t>(request_timespec_addr)?;

    debug!(
        "clockid = {:?}, abs_time = {}, request_timespec = {:?}, remain timespec addr = 0x{:x}",
        clock_id, abs_time, request_timespec, remain_timespec_addr
    );
    // FIXME: do real sleep. Here we simply yield the execution of current thread since we does not have timeout support now.
    // If the sleep is interrupted by a signal, this syscall should return error.
    Thread::yield_now();
    if remain_timespec_addr != 0 {
        let remain_duration = Duration::new(0, 0);
        let remain_timespec = timespec_t::from(remain_duration);
        write_val_to_user(remain_timespec_addr, &remain_timespec)?;
    }

    Ok(SyscallReturn::Return(0))
}
