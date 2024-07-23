// SPDX-License-Identifier: MPL-2.0

//! The system time of Asterinas.
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::sync::Arc;
use core::{sync::atomic::Ordering::Relaxed, time::Duration};

use chrono::{DateTime, Datelike, Timelike};
use clocksource::ClockSource;
pub use clocksource::Instant;
use component::{init_component, ComponentInitError};
use ostd::{io_mem::IoMem, mm::VmIo, sync::Mutex};
use spin::Once;

mod clocksource;
mod tsc;

pub const NANOS_PER_SECOND: u32 = 1_000_000_000;
pub static VDSO_DATA_HIGH_RES_UPDATE_FN: Once<Arc<dyn Fn(Instant, u64) + Sync + Send>> =
    Once::new();

#[init_component]
fn time_init() -> Result<(), ComponentInitError> {
    tsc::init();
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SystemTime {
    century: u8,
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
            century: 0,
            year: 0,
            month: 0,
            day: 0,
            hour: 0,
            minute: 0,
            second: 0,
            nanos: 0,
        }
    }

    pub(crate) fn update_from_rtc(&mut self) {
        const TIME_LOW: usize = 0;
        const TIME_HIGH: usize = 4;

        let io_mem = ostd::arch::riscv::timer::GOLDFISH_IO_MEM.get().unwrap();

        let mut last_time_high = io_mem.read_val(TIME_HIGH).unwrap();
        let timestamp = loop {
            let time_low: u32 = io_mem.read_val(TIME_LOW).unwrap();
            let time_high: u32 = io_mem.read_val(TIME_HIGH).unwrap();
            if last_time_high == time_high {
                break ((time_high as u64) << 32) | time_low as u64;
            }
            last_time_high = time_high;
        };

        let time = DateTime::from_timestamp_nanos(timestamp as i64).naive_utc();
        self.second = time.second() as u8;
        self.minute = time.minute() as u8;
        self.hour = time.hour() as u8;
        self.day = time.day() as u8;
        self.month = time.month() as u8;

        let (is_ad, year) = time.year_ce();
        debug_assert!(is_ad, "non-negative timestamp should always be AD");
        self.year = year as u16;
        self.century = (year / 100) as u8;
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
    READ_TIME.lock().update_from_rtc();
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
