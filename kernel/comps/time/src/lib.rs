// SPDX-License-Identifier: MPL-2.0

//! The system time of Asterinas.
#![feature(let_chains)]
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::sync::Arc;
use core::time::Duration;

use clocksource::ClockSource;
pub use clocksource::Instant;
use component::{init_component, ComponentInitError};
use ostd::sync::Mutex;
use rtc::Driver;
use spin::Once;

mod clocksource;
mod rtc;
pub mod tsc;

pub const NANOS_PER_SECOND: u32 = 1_000_000_000;
pub static VDSO_DATA_HIGH_RES_UPDATE_FN: Once<Arc<dyn Fn(Instant, u64) + Sync + Send>> =
    Once::new();
static RTC_DRIVER: Once<Arc<dyn Driver + Send + Sync>> = Once::new();

#[init_component]
fn time_init() -> Result<(), ComponentInitError> {
    let rtc = rtc::init_rtc_driver().ok_or(ComponentInitError::Unknown)?;
    RTC_DRIVER.call_once(|| rtc);
    tsc::init();
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SystemTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub nanos: u64,
}

impl SystemTime {
    pub(crate) const fn zero() -> Self {
        Self {
            year: 0,
            month: 0,
            day: 0,
            hour: 0,
            minute: 0,
            second: 0,
            nanos: 0,
        }
    }
}

pub(crate) static READ_TIME: Mutex<SystemTime> = Mutex::new(SystemTime::zero());
pub(crate) static START_TIME: Once<SystemTime> = Once::new();

/// get real time
pub fn get_real_time() -> SystemTime {
    read()
}

pub fn read() -> SystemTime {
    update_time();
    *READ_TIME.lock()
}

fn update_time() {
    let mut lock = READ_TIME.lock();
    *lock = RTC_DRIVER.get().unwrap().read_rtc();
}

/// Return the `START_TIME`, which is the actual time when doing calibrate.
pub fn read_start_time() -> SystemTime {
    *START_TIME.get().unwrap()
}

/// Return the monotonic time from the tsc clocksource.
pub fn read_monotonic_time() -> Duration {
    let instant = tsc::read_instant();
    Duration::new(instant.secs(), instant.nanos())
}

/// Return the tsc clocksource.
pub fn default_clocksource() -> Arc<ClockSource> {
    tsc::CLOCK.get().unwrap().clone()
}
