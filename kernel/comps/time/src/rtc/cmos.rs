// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU8, Ordering::Relaxed};

use ostd::arch::x86::device::cmos::{century_register, CMOS_ADDRESS, CMOS_DATA};

use crate::SystemTime;
use super::Driver;

static CENTURY_REGISTER: AtomicU8 = AtomicU8::new(0);

fn get_cmos(reg: u8) -> u8 {
    CMOS_ADDRESS.write(reg);
    CMOS_DATA.read()
}

fn is_updating() -> bool {
    CMOS_ADDRESS.write(0x0A);
    CMOS_DATA.read() & 0x80 != 0
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CmosData {
    century: u8,
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
}

impl CmosData {
    fn from_rtc_raw(century_register: u8) -> Self {
        while is_updating() {}

        let second = get_cmos(0x00);
        let minute = get_cmos(0x02);
        let hour = get_cmos(0x04);
        let day = get_cmos(0x07);
        let month = get_cmos(0x08);
        let year = get_cmos(0x09) as u16;

        let century = if century_register != 0 {
            get_cmos(century_register)
        } else {
            0
        };

        CmosData {
            century,
            year,
            month,
            day,
            hour,
            minute,
            second,
        }
    }

    /// Converts BCD to binary values.
    /// ref: https://wiki.osdev.org/CMOS#Reading_All_RTC_Time_and_Date_Registers
    fn convert_bcd_to_binary(&mut self, register_b: u8) {
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

    /// Converts 12 hour clock to 24 hour clock.
    fn convert_12_hour_to_24_hour(&mut self, register_b: u8) {
        // bit1 in register_b is not set if 12 hour format is enable
        // if highest bit in hour is set, then it is pm
        if ((register_b & 0x02) == 0) && ((self.hour & 0x80) != 0) {
            self.hour = ((self.hour & 0x7F) + 12) % 24;
        }
    }

    /// Converts raw year (10, 20 etc.) to real year (2010, 2020 etc.).
    fn modify_year(&mut self) {
        self.year += self.century as u16 * 100;
    }

    pub fn read_rtc(century_register: u8) -> Self {
        let mut now = Self::from_rtc_raw(century_register);
        while let new = Self::from_rtc_raw(century_register) && now != new {
            now = new;
        }

        let register_b: u8 = get_cmos(0x0B);

        now.convert_bcd_to_binary(register_b);
        now.convert_12_hour_to_24_hour(register_b);
        now.modify_year();

        now
    }
}

impl From<CmosData> for SystemTime {
    fn from(cmos: CmosData) -> SystemTime {
        SystemTime {
            year: cmos.year,
            month: cmos.month,
            day: cmos.day,
            hour: cmos.hour,
            minute: cmos.minute,
            second: cmos.second,
            nanos: 0,
        }
    }
}

pub struct RtcCmos {
    century_register: u8,
}

impl Driver for RtcCmos {
    fn try_new() -> Option<RtcCmos> {
        Some(RtcCmos {
            century_register: century_register().unwrap_or(0),
        })
    }

    fn read_rtc(&self) -> SystemTime {
        CmosData::read_rtc(self.century_register).into()
    }
}
