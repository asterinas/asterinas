// SPDX-License-Identifier: MPL-2.0

#![allow(non_camel_case_types)]

pub use core::{timer, Clock};

use ::core::time::Duration;
pub use system_time::{SystemTime, START_TIME};
pub use timer::{Timer, TimerManager};

use crate::prelude::*;

pub mod clocks;
mod core;
mod softirq;
mod system_time;
pub mod wait;

pub type clockid_t = i32;
pub type time_t = i64;
pub type suseconds_t = i64;
pub type clock_t = i64;

const NSEC_PER_USEC: i64 = 1_000;
const USEC_PER_SEC: i64 = 1_000_000;
const NSEC_PER_SEC: i64 = 1_000_000_000;

pub(super) fn init() {
    system_time::init();
    clocks::init();
    softirq::init();
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

impl From<timeval_t> for timespec_t {
    fn from(timeval: timeval_t) -> timespec_t {
        let sec = timeval.sec;
        let nsec = timeval.usec * NSEC_PER_USEC;
        debug_assert!(sec >= 0); // nsec >= 0 always holds
        timespec_t { sec, nsec }
    }
}

impl TryFrom<timespec_t> for Duration {
    type Error = crate::Error;

    fn try_from(value: timespec_t) -> Result<Self> {
        if value.sec < 0 || value.nsec < 0 {
            return_errno_with_message!(Errno::EINVAL, "timesepc_t cannot be negative");
        }

        if value.nsec > NSEC_PER_SEC {
            // The value of nanoseconds cannot exceed 10^9,
            // otherwise the value for seconds should be set.
            return_errno_with_message!(Errno::EINVAL, "nsec is not normalized");
        }

        Ok(Duration::new(value.sec as u64, value.nsec as u32))
    }
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone, Pod)]
pub struct timeval_t {
    pub sec: time_t,
    pub usec: suseconds_t,
}

impl timeval_t {
    /// Normalizes time by adding carries from microseconds to seconds.
    ///
    /// Some Linux system calls do this before checking the validity (e.g., the [select]
    /// implementation).
    ///
    /// [select]: https://elixir.bootlin.com/linux/v6.10.5/source/fs/select.c#L716
    pub fn normalize(&self) -> Self {
        Self {
            sec: self.sec.wrapping_add(self.usec / USEC_PER_SEC),
            usec: self.usec % USEC_PER_SEC,
        }
    }
}

impl From<Duration> for timeval_t {
    fn from(duration: Duration) -> timeval_t {
        let sec = duration.as_secs() as time_t;
        let usec = duration.subsec_micros() as suseconds_t;
        debug_assert!(sec >= 0); // usec >= 0 always holds
        timeval_t { sec, usec }
    }
}

impl TryFrom<timeval_t> for Duration {
    type Error = crate::Error;

    fn try_from(timeval: timeval_t) -> Result<Self> {
        if timeval.sec < 0 || timeval.usec < 0 {
            return_errno_with_message!(Errno::EINVAL, "timeval_t cannot be negative");
        }
        if timeval.usec > USEC_PER_SEC {
            // The value of microsecond cannot exceed 10^6,
            // otherwise the value for seconds should be set.
            return_errno_with_message!(Errno::EINVAL, "nsec is not normalized");
        }

        Ok(Duration::new(
            timeval.sec as u64,
            (timeval.usec * NSEC_PER_USEC) as u32,
        ))
    }
}

/// The various flags for setting POSIX.1b interval timers:
pub const TIMER_ABSTIME: i32 = 0x01;

/// Unix time measures time by the number of seconds that have elapsed since
/// the Unix epoch, without adjustments made due to leap seconds.
#[repr(C)]
#[derive(Debug, Default, Copy, Clone, Pod)]
pub struct UnixTime {
    sec: u32,
}

impl From<Duration> for UnixTime {
    fn from(duration: Duration) -> Self {
        Self {
            sec: duration.as_secs() as u32,
        }
    }
}

impl From<UnixTime> for Duration {
    fn from(time: UnixTime) -> Self {
        Duration::from_secs(time.sec as _)
    }
}

/// This struct is corresponding to the `itimerval` struct in Linux.
#[repr(C)]
#[derive(Debug, Default, Copy, Clone, Pod)]
pub struct itimerval_t {
    pub it_interval: timeval_t,
    pub it_value: timeval_t,
}

/// This struct is corresponding to the `itimerspec` struct in Linux.
#[repr(C)]
#[derive(Debug, Default, Copy, Clone, Pod)]
pub struct itimerspec_t {
    pub it_interval: timespec_t,
    pub it_value: timespec_t,
}
