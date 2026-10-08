// SPDX-License-Identifier: MPL-2.0

//! MC146818 calendar snapshots and whole-second write transactions.

use core::num::NonZeroU8;

use crate::{SystemTime, rtc::RtcError};

pub(super) trait Access {
    fn read_register(&mut self, register: u8) -> u8;
    fn write_register(&mut self, register: u8, value: u8);
    fn century_register(&self) -> Option<NonZeroU8>;
    fn divider_mode(&self) -> DividerMode;
}

#[derive(Clone, Copy)]
pub(super) enum DividerMode {
    Standard,
    Amd,
}

#[repr(u8)]
#[derive(Clone, Copy)]
pub(super) enum Register {
    Second = 0x00,
    Minute = 0x02,
    Hour = 0x04,
    Day = 0x07,
    Month = 0x08,
    Year = 0x09,
    StatusA = 0x0a,
    StatusB = 0x0b,
    StatusD = 0x0d,
}

const UIP: u8 = 1 << 7;
const SET: u8 = 1 << 7;
pub(super) const VRT: u8 = 1 << 7;
const BINARY: u8 = 1 << 2;
const HOUR_24: u8 = 1 << 1;
const PM: u8 = 1 << 7;
const DIVIDER_RESET: u8 = 0x70;
const AMD_BANK_SELECT: u8 = 1 << 4;

// Order matches the decoded SystemTime fields below.
const CALENDAR_REGISTERS: [Register; 6] = [
    Register::Year,
    Register::Month,
    Register::Day,
    Register::Hour,
    Register::Minute,
    Register::Second,
];

fn read_raw(access: &mut impl Access) -> [u8; 7] {
    let mut raw = [0; 7];
    for (byte, register) in raw.iter_mut().zip(CALENDAR_REGISTERS) {
        *byte = access.read_register(register as u8);
    }
    if let Some(register) = access.century_register() {
        raw[6] = access.read_register(register.get());
    }
    raw
}

pub(super) fn read(access: &mut impl Access) -> Result<Option<SystemTime>, RtcError> {
    if access.read_register(Register::StatusD as u8) != VRT {
        return Err(RtcError::InvalidTime);
    }
    // Check seconds both before and after the UIP checks. A long interruption
    // can otherwise hide an entire update cycle, as noted by Linux's
    // mc146818_avoid_UIP (drivers/rtc/rtc-mc146818-lib.c).
    let second = access.read_register(Register::Second as u8);
    if access.read_register(Register::StatusA as u8) & UIP != 0 {
        return Ok(None);
    }
    let status_b = access.read_register(Register::StatusB as u8);
    if status_b & SET != 0 {
        return Ok(None);
    }
    let raw = read_raw(access);
    let next = read_raw(access);
    if raw != next
        || access.read_register(Register::StatusB as u8) != status_b
        || access.read_register(Register::StatusA as u8) & UIP != 0
        || access.read_register(Register::Second as u8) != second
    {
        return Ok(None);
    }

    let decode = |value| decode_byte(value, status_b);
    let year = u16::from(decode(raw[0])?);
    let century = if access.century_register().is_some() {
        u16::from(decode(raw[6])?)
    } else {
        20
    };
    if year > 99 || century > 99 {
        return Err(RtcError::InvalidTime);
    }
    let hour = if status_b & HOUR_24 != 0 {
        decode(raw[3])?
    } else {
        let hour = decode(raw[3] & !PM)?;
        if !(1..=12).contains(&hour) {
            return Err(RtcError::InvalidTime);
        }
        hour % 12 + if raw[3] & PM != 0 { 12 } else { 0 }
    };
    let time = SystemTime {
        year: century * 100 + year,
        month: decode(raw[1])?,
        day: decode(raw[2])?,
        hour,
        minute: decode(raw[4])?,
        second: decode(raw[5])?,
        nanos: 0,
    };
    validate(&time)?;
    Ok(Some(time))
}

pub(super) fn write(access: &mut impl Access, time: &SystemTime) -> Result<Option<()>, RtcError> {
    // Reject all invalid inputs before touching writable registers.
    validate(time)?;
    if time.nanos != 0
        || (access.century_register().is_none() && !(2000..=2099).contains(&time.year))
    {
        return Err(RtcError::InvalidTime);
    }
    if access.read_register(Register::StatusD as u8) != VRT {
        return Err(RtcError::InvalidTime);
    }
    let status_a = access.read_register(Register::StatusA as u8);
    let status_b = access.read_register(Register::StatusB as u8);
    if status_a & UIP != 0 || status_b & SET != 0 {
        return Ok(None);
    }

    let encode = |value| encode_byte(value, status_b);
    let hour = if status_b & HOUR_24 != 0 {
        encode(time.hour)
    } else {
        let hour = match time.hour % 12 {
            0 => 12,
            hour => hour,
        };
        encode(hour) | if time.hour >= 12 { PM } else { 0 }
    };
    let raw = [
        encode((time.year % 100) as u8),
        encode(time.month),
        encode(time.day),
        hour,
        encode(time.minute),
        encode(time.second),
    ];

    // Follow mc146818_set_time: inhibit calendar updates, reset the divider,
    // write the calendar, then restore B before A. AMD/Hygon uses DV1 as a
    // bank selector, so clear that bit instead of resetting the divider.
    // No fallible operation remains after updates are inhibited.
    access.write_register(Register::StatusB as u8, status_b | SET);
    let inhibited_a = match access.divider_mode() {
        DividerMode::Standard => status_a | DIVIDER_RESET,
        DividerMode::Amd => status_a & !AMD_BANK_SELECT,
    };
    access.write_register(Register::StatusA as u8, inhibited_a);
    for (register, value) in CALENDAR_REGISTERS.into_iter().zip(raw) {
        access.write_register(register as u8, value);
    }
    if let Some(register) = access.century_register() {
        access.write_register(register.get(), encode((time.year / 100) as u8));
    }
    access.write_register(Register::StatusB as u8, status_b);
    access.write_register(Register::StatusA as u8, status_a);
    Ok(Some(()))
}

fn decode_byte(value: u8, status_b: u8) -> Result<u8, RtcError> {
    if status_b & BINARY != 0 {
        return Ok(value);
    }
    let high = value >> 4;
    let low = value & 0x0f;
    if high > 9 || low > 9 {
        return Err(RtcError::InvalidTime);
    }
    Ok(high * 10 + low)
}

fn encode_byte(value: u8, status_b: u8) -> u8 {
    if status_b & BINARY != 0 {
        value
    } else {
        (value / 10) << 4 | (value % 10)
    }
}

fn validate(time: &SystemTime) -> Result<(), RtcError> {
    let days = match time.month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            let is_leap = time.year.is_multiple_of(4)
                && (!time.year.is_multiple_of(100) || time.year.is_multiple_of(400));
            if is_leap { 29 } else { 28 }
        }
        _ => return Err(RtcError::InvalidTime),
    };
    // Boot timekeeping represents time as a duration since the Unix epoch.
    if !(1970..=9999).contains(&time.year)
        || !(1..=days).contains(&time.day)
        || time.hour >= 24
        || time.minute >= 60
        || time.second >= 60
        || time.nanos >= 1_000_000_000
    {
        return Err(RtcError::InvalidTime);
    }
    Ok(())
}

#[cfg(ktest)]
mod test;
