// SPDX-License-Identifier: MPL-2.0

//! The system time of Asterinas.

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::sync::Arc;
use core::time::Duration;

pub use clocksource::{ClockSource, Instant};
use component::{ComponentInitError, init_component};
use rtc::Driver;
pub use rtc::{RtcAlarm, RtcAlarmCallback, RtcAlarmState, RtcAlarmTime, RtcError};
use spin::Once;

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "time: "
    };
}

mod clocksource;
mod rtc;
mod tsc;

pub static VDSO_DATA_HIGH_RES_UPDATE_FN: Once<fn(Instant, u64)> = Once::new();

static RTC_DRIVER: Once<Arc<dyn Driver + Send + Sync>> = Once::new();

#[init_component]
fn time_init() -> Result<(), ComponentInitError> {
    let rtc = rtc::init_rtc_driver();
    RTC_DRIVER.call_once(|| rtc);
    tsc::init();
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SystemTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub nanos: u64,
}

#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
impl From<chrono::NaiveDateTime> for SystemTime {
    fn from(time: chrono::NaiveDateTime) -> Self {
        use chrono::{Datelike, Timelike};

        Self {
            year: time.year() as u16,
            month: time.month() as u8,
            day: time.day() as u8,
            hour: time.hour() as u8,
            minute: time.minute() as u8,
            second: time.second() as u8,
            nanos: time.nanosecond() as u64,
        }
    }
}

static START_TIME: Once<SystemTime> = Once::new();

/// Returns the `START_TIME`, which is the system time when calibrating.
pub fn read_start_time() -> SystemTime {
    *START_TIME.get().unwrap()
}

/// Reads the current hardware RTC time.
///
/// Returns [`RtcError::NotInitialized`] before component initialization and
/// [`RtcError::NoDevice`] if no hardware RTC was found. This reads the RTC
/// independently of the system's wall clock and the saved start time.
pub fn read_rtc() -> Result<SystemTime, RtcError> {
    RTC_DRIVER.get().ok_or(RtcError::NotInitialized)?.read_rtc()
}

/// Sets the hardware RTC time.
///
/// This does not change the system's wall clock or the saved start time.
/// The CMOS backend accepts whole seconds and years 1970 through 9999 only.
/// Without a century register, its supported years are 2000 through 2099.
/// Other hardware backends currently return
/// [`RtcError::Unsupported`]. Authorization is the caller's responsibility.
pub fn set_rtc(time: SystemTime) -> Result<(), RtcError> {
    RTC_DRIVER
        .get()
        .ok_or(RtcError::NotInitialized)?
        .set_rtc(&time)
}

/// Reads the hardware alarm configuration and its latched pending state.
///
/// Reading does not acknowledge or consume an alarm event.
pub fn read_rtc_alarm() -> Result<RtcAlarm, RtcError> {
    RTC_DRIVER
        .get()
        .ok_or(RtcError::NotInitialized)?
        .read_alarm()
}

/// Configures a time-of-day alarm and clears the previous pending event.
///
/// The alarm is disabled after its next match. Date matching and system wakeup
/// are not supported. Authorization is the caller's responsibility.
/// If a write times out after starting, the alarm remains disabled and its
/// match fields may have changed. A failed operation does not restore the old alarm.
pub fn set_rtc_alarm(time: RtcAlarmTime, state: RtcAlarmState) -> Result<(), RtcError> {
    RTC_DRIVER
        .get()
        .ok_or(RtcError::NotInitialized)?
        .set_alarm(time, state)
}

/// Arms or cancels the configured alarm.
///
/// Arming clears the previous pending event. Cancelling preserves an event that
/// has already been delivered. Authorization is the caller's responsibility.
pub fn set_rtc_alarm_state(state: RtcAlarmState) -> Result<(), RtcError> {
    RTC_DRIVER
        .get()
        .ok_or(RtcError::NotInitialized)?
        .set_alarm_state(state)
}

/// Replaces the RTC alarm notification callback, or removes it with `None`.
///
/// There is one callback slot per RTC. Removal does not wait for a notification
/// already in progress. The callback runs in interrupt context; it must not sleep.
/// A latched pending event remains available through [`take_rtc_alarm_event`].
pub fn set_rtc_alarm_callback(callback: Option<Arc<RtcAlarmCallback>>) -> Result<(), RtcError> {
    RTC_DRIVER
        .get()
        .ok_or(RtcError::NotInitialized)?
        .set_alarm_callback(callback)
}

/// Consumes the latched alarm event, returning whether one was pending.
pub fn take_rtc_alarm_event() -> Result<bool, RtcError> {
    RTC_DRIVER
        .get()
        .ok_or(RtcError::NotInitialized)?
        .take_alarm_event()
}

/// Returns the monotonic time from the TSC clocksource.
pub fn read_monotonic_time() -> Duration {
    let instant = tsc::read_instant();
    Duration::new(instant.secs(), instant.nanos())
}

/// Returns the default (TSC) clocksource.
pub fn default_clocksource() -> Arc<ClockSource> {
    tsc::CLOCK.get().unwrap().clone()
}
