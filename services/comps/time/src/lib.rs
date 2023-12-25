//! The system time of Asterinas.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::sync::Arc;
use aster_frame::sync::Mutex;
use component::{init_component, ComponentInitError};
use core::{sync::atomic::Ordering::Relaxed, time::Duration};
use spin::Once;

use clocksource::ClockSource;
use rtc::{get_cmos, is_updating, CENTURY_REGISTER};

pub use clocksource::Instant;

mod clocksource;
mod rtc;
mod tsc;

pub const NANOS_PER_SECOND: u32 = 1_000_000_000;
pub static VDSO_DATA_UPDATE: Once<Arc<dyn Fn(Instant, u64) + Sync + Send>> = Once::new();

#[init_component]
fn time_init() -> Result<(), ComponentInitError> {
    rtc::init();
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
        while is_updating() {}
        self.second = get_cmos(0x00);
        self.minute = get_cmos(0x02);
        self.hour = get_cmos(0x04);
        self.day = get_cmos(0x07);
        self.month = get_cmos(0x08);
        self.year = get_cmos(0x09) as u16;

        let century_register = CENTURY_REGISTER.load(Relaxed);
        if century_register != 0 {
            self.century = get_cmos(century_register);
        }
    }

    /// convert BCD to binary values
    /// ref:https://wiki.osdev.org/CMOS#Reading_All_RTC_Time_and_Date_Registers
    pub(crate) fn convert_bcd_to_binary(&mut self, register_b: u8) {
        if register_b & 0x04 == 0 {
            self.second = (self.second & 0x0F) + ((self.second / 16) * 10);
            self.minute = (self.minute & 0x0F) + ((self.minute / 16) * 10);
            self.hour =
                ((self.hour & 0x0F) + (((self.hour & 0x70) / 16) * 10)) | (self.hour & 0x80);
            self.day = (self.day & 0x0F) + ((self.day / 16) * 10);
            self.month = (self.month & 0x0F) + ((self.month / 16) * 10);
            self.year = (self.year & 0x0F) + ((self.year / 16) * 10);
            if CENTURY_REGISTER.load(Relaxed) != 0 {
                self.century = (self.century & 0x0F) + ((self.century / 16) * 10);
            } else {
                // 2000 ~ 2099
                const DEFAULT_21_CENTURY: u8 = 20;
                self.century = DEFAULT_21_CENTURY;
            }
        }
    }
    /// convert 12 hour clock to 24 hour clock
    pub(crate) fn convert_12_hour_to_24_hour(&mut self, register_b: u8) {
        // bit1 in register_b is not set if 12 hour format is enable
        // if highest bit in hour is set, then it is pm
        if ((register_b & 0x02) == 0) && ((self.hour & 0x80) != 0) {
            self.hour = ((self.hour & 0x7F) + 12) % 24;
        }
    }

    /// convert raw year (10, 20 etc.) to real year (2010, 2020 etc.)
    pub(crate) fn modify_year(&mut self) {
        self.year += self.century as u16 * 100;
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

/// read year,month,day and other data
/// ref: https://wiki.osdev.org/CMOS#Reading_All_RTC_Time_and_Date_Registers
fn update_time() {
    let mut last_time: SystemTime;

    let mut lock = READ_TIME.lock();

    lock.update_from_rtc();

    last_time = *lock;

    lock.update_from_rtc();

    while *lock != last_time {
        last_time = *lock;
        lock.update_from_rtc();
    }
    let register_b: u8 = get_cmos(0x0B);

    lock.convert_bcd_to_binary(register_b);
    lock.convert_12_hour_to_24_hour(register_b);
    lock.modify_year();
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
