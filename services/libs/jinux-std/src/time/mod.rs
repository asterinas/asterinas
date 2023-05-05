#![allow(non_camel_case_types)]
use core::time::Duration;

use crate::prelude::*;

mod system_time;
pub use system_time::SystemTime;

pub type clockid_t = i32;
pub type time_t = i64;
pub type suseconds_t = i64;
pub type clock_t = i64;

#[derive(Debug, Copy, Clone, TryFromInt)]
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

#[repr(C)]
#[derive(Debug, Default, Copy, Clone, Pod)]
pub struct timeval_t {
    pub sec: time_t,
    pub usec: suseconds_t,
}

impl From<Duration> for timeval_t {
    fn from(duration: Duration) -> timeval_t {
        let sec = duration.as_secs() as time_t;
        let usec = duration.subsec_micros() as suseconds_t;
        debug_assert!(sec >= 0); // usec >= 0 always holds
        timeval_t { sec, usec }
    }
}

impl From<timeval_t> for Duration {
    fn from(timeval: timeval_t) -> Self {
        Duration::new(timeval.sec as u64, (timeval.usec * 1000) as u32)
    }
}

/// The various flags for setting POSIX.1b interval timers:
pub const TIMER_ABSTIME: i32 = 0x01;
