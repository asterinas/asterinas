// SPDX-License-Identifier: MPL-2.0

#![allow(non_camel_case_types)]
use core::time::Duration;

use int_to_c_enum::TryFromInt;

use super::SyscallReturn;
use crate::{
    prelude::*,
    time::{
        clockid_t,
        clocks::{
            BootTimeClock, MonotonicClock, MonotonicCoarseClock, MonotonicRawClock, RealTimeClock,
            RealTimeCoarseClock,
        },
        timespec_t, Clock,
    },
    util::write_val_to_user,
};

pub fn sys_clock_gettime(clockid: clockid_t, timespec_addr: Vaddr) -> Result<SyscallReturn> {
    let clock_id = ClockID::try_from(clockid)?;
    debug!("clockid = {:?}", clock_id);

    let time_duration = read_clock(&clock_id)?;

    let timespec = timespec_t::from(time_duration);
    write_val_to_user(timespec_addr, &timespec)?;

    Ok(SyscallReturn::Return(0))
}

#[derive(Debug, Copy, Clone, TryFromInt, PartialEq)]
#[repr(i32)]
pub enum ClockID {
    CLOCK_REALTIME = 0,
    CLOCK_MONOTONIC = 1,
    CLOCK_PROCESS_CPUTIME_ID = 2,
    CLOCK_THREAD_CPUTIME_ID = 3,
    CLOCK_MONOTONIC_RAW = 4,
    CLOCK_REALTIME_COARSE = 5,
    CLOCK_MONOTONIC_COARSE = 6,
    CLOCK_BOOTTIME = 7,
}

/// Read the time of a clock specified by the input `ClockID`.
///
/// If the `ClockID` does not support, this function will return `Err`.
pub fn read_clock(clock_id: &ClockID) -> Result<Duration> {
    match clock_id {
        ClockID::CLOCK_REALTIME => Ok(RealTimeClock::get().read_time()),
        ClockID::CLOCK_MONOTONIC => Ok(MonotonicClock::get().read_time()),
        ClockID::CLOCK_MONOTONIC_RAW => Ok(MonotonicRawClock::get().read_time()),
        ClockID::CLOCK_REALTIME_COARSE => Ok(RealTimeCoarseClock::get().read_time()),
        ClockID::CLOCK_MONOTONIC_COARSE => Ok(MonotonicCoarseClock::get().read_time()),
        ClockID::CLOCK_BOOTTIME => Ok(BootTimeClock::get().read_time()),
        _ => {
            return_errno_with_message!(Errno::EINVAL, "unsupported clock_id");
        }
    }
}
