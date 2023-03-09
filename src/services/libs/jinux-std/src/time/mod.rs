#![allow(non_camel_case_types)]
use core::time::Duration;

use crate::prelude::*;

pub type clockid_t = i32;
pub type time_t = i64;
pub type suseconds_t = i64;
pub type clock_t = i64;

#[derive(Debug, Copy, Clone, Pod)]
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

impl TryFrom<clockid_t> for ClockID {
    type Error = Error;

    fn try_from(value: clockid_t) -> Result<Self> {
        Ok(match value as i32 {
            0 => ClockID::CLOCK_REALTIME,
            1 => ClockID::CLOCK_MONOTONIC,
            2 => ClockID::CLOCK_PROCESS_CPUTIME_ID,
            3 => ClockID::CLOCK_THREAD_CPUTIME_ID,
            4 => ClockID::CLOCK_MONOTONIC_RAW,
            5 => ClockID::CLOCK_REALTIME_COARSE,
            6 => ClockID::CLOCK_MONOTONIC_COARSE,
            7 => ClockID::CLOCK_BOOTTIME,
            _ => return_errno_with_message!(Errno::EINVAL, "invalid clockid"),
        })
    }
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone, Pod)]
pub struct timespec_t {
    pub sec: time_t,
    pub nsec: i64,
}

impl From<Duration> for timespec_t {
    fn from(duration: Duration) -> timespec_t {
        let sec = duration.as_secs() as time_t;
        let nsec = duration.subsec_nanos() as i64;
        debug_assert!(sec >= 0); // nsec >= 0 always holds
        timespec_t { sec, nsec }
    }
}

impl From<timespec_t> for Duration {
    fn from(timespec: timespec_t) -> Self {
        Duration::new(timespec.sec as u64, timespec.nsec as u32)
    }
}

/// The various flags for setting POSIX.1b interval timers:
pub const TIMER_ABSTIME: i32 = 0x01;
