// SPDX-License-Identifier: MPL-2.0

//! Legacy time-of-day alarm transactions. Status C reads acknowledge all sources.

use super::calendar::{Access, HOUR_24, PM, Register, SET, UIP, decode_byte, encode_byte};
use crate::{RtcAlarm, RtcAlarmState, RtcAlarmTime, RtcError};

pub(super) const AIE: u8 = 0x20;
const IRQF: u8 = 0x80;
const AF: u8 = 0x20;
const REGISTERS: [Register; 3] = [
    Register::HourAlarm,
    Register::MinuteAlarm,
    Register::SecondAlarm,
];

fn raw(access: &mut impl Access) -> [u8; 3] {
    REGISTERS.map(|register| access.read_register(register as u8))
}

fn decode(value: u8, status_b: u8, hour: bool) -> Result<Option<u8>, RtcError> {
    if value & 0xc0 == 0xc0 {
        return Ok(None);
    }
    let value = if hour && status_b & HOUR_24 == 0 {
        let decoded = decode_byte(value & !PM, status_b)?;
        if !(1..=12).contains(&decoded) {
            return Err(RtcError::InvalidTime);
        }
        decoded % 12 + if value & PM != 0 { 12 } else { 0 }
    } else {
        decode_byte(value, status_b)?
    };
    if value >= if hour { 24 } else { 60 } {
        return Err(RtcError::InvalidTime);
    }
    Ok(Some(value))
}

fn encode(value: Option<u8>, status_b: u8, hour: bool) -> u8 {
    let Some(value) = value else { return 0xff };
    if hour && status_b & HOUR_24 == 0 {
        encode_byte(if value % 12 == 0 { 12 } else { value % 12 }, status_b)
            | if value >= 12 { PM } else { 0 }
    } else {
        encode_byte(value, status_b)
    }
}

pub(super) fn read(access: &mut impl Access, pending: bool) -> Result<Option<RtcAlarm>, RtcError> {
    let second = access.read_register(Register::Second as u8);
    if access.read_register(Register::StatusA as u8) & UIP != 0 {
        return Ok(None);
    }
    let status_b = access.read_register(Register::StatusB as u8);
    if status_b & SET != 0 {
        return Ok(None);
    }
    let values = raw(access);
    if values != raw(access)
        || access.read_register(Register::StatusB as u8) != status_b
        || access.read_register(Register::StatusA as u8) & UIP != 0
        || access.read_register(Register::Second as u8) != second
    {
        return Ok(None);
    }
    Ok(Some(RtcAlarm {
        time: RtcAlarmTime {
            hour: decode(values[0], status_b, true)?,
            minute: decode(values[1], status_b, false)?,
            second: decode(values[2], status_b, false)?,
        },
        state: if status_b & AIE != 0 {
            RtcAlarmState::Enabled
        } else {
            RtcAlarmState::Disabled
        },
        pending,
    }))
}

pub(super) fn write(
    access: &mut impl Access,
    time: RtcAlarmTime,
    state: RtcAlarmState,
) -> Result<Option<()>, RtcError> {
    if time.hour.is_some_and(|v| v >= 24)
        || time.minute.is_some_and(|v| v >= 60)
        || time.second.is_some_and(|v| v >= 60)
    {
        return Err(RtcError::InvalidTime);
    }
    let second = access.read_register(Register::Second as u8);
    if access.read_register(Register::StatusA as u8) & UIP != 0 {
        return Ok(None);
    }
    let status_b = access.read_register(Register::StatusB as u8);
    if status_b & SET != 0 {
        return Ok(None);
    }
    access.write_register(Register::StatusB as u8, status_b & !AIE);
    access.read_register(Register::StatusC as u8);
    let values = [
        encode(time.hour, status_b, true),
        encode(time.minute, status_b, false),
        encode(time.second, status_b, false),
    ];
    for (register, value) in REGISTERS.into_iter().zip(values) {
        access.write_register(register as u8, value);
    }
    // Alarm registers may be disconnected during UIP. Check the update boundary
    // and read back writes; retry with AIE disabled if an update overlapped us.
    if access.read_register(Register::StatusA as u8) & UIP != 0
        || access.read_register(Register::Second as u8) != second
        || raw(access) != values
    {
        return Ok(None);
    }
    // Discard matches latched while configuring, before arming the new alarm.
    access.read_register(Register::StatusC as u8);
    access.write_register(
        Register::StatusB as u8,
        if state == RtcAlarmState::Enabled {
            status_b | AIE
        } else {
            status_b & !AIE
        },
    );
    Ok(Some(()))
}

pub(super) fn set_state(
    access: &mut impl Access,
    state: RtcAlarmState,
) -> Result<Option<()>, RtcError> {
    if state == RtcAlarmState::Enabled && read(access, false)?.is_none() {
        return Ok(None);
    }
    let status_b = access.read_register(Register::StatusB as u8);
    access.write_register(Register::StatusB as u8, status_b & !AIE);
    access.read_register(Register::StatusC as u8);
    if state == RtcAlarmState::Enabled {
        access.write_register(Register::StatusB as u8, status_b | AIE);
    }
    Ok(Some(()))
}

/// Acknowledge every hardware source, but deliver only an armed alarm once.
pub(super) fn acknowledge(access: &mut impl Access) -> bool {
    let status_c = access.read_register(Register::StatusC as u8);
    let status_b = access.read_register(Register::StatusB as u8);
    if status_c & (IRQF | AF) != IRQF | AF || status_b & AIE == 0 {
        return false;
    }
    access.write_register(Register::StatusB as u8, status_b & !AIE);
    true
}

#[cfg(ktest)]
mod test;
