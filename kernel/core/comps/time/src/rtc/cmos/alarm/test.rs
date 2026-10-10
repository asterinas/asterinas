// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;
use core::num::NonZeroU8;

use ostd::prelude::ktest;

use super::*;
use crate::rtc::cmos::calendar::DividerMode;

struct Memory {
    bytes: [u8; 128],
    writes: Vec<(u8, u8)>,
    c_reads: usize,
    a_reads: usize,
    late_uip: bool,
    change_second: bool,
}

impl Memory {
    fn new(mode: u8) -> Self {
        let mut bytes = [0; 128];
        bytes[Register::StatusB as usize] = mode;
        Self {
            bytes,
            writes: Vec::new(),
            c_reads: 0,
            a_reads: 0,
            late_uip: false,
            change_second: false,
        }
    }
}

impl Access for Memory {
    fn read_register(&mut self, register: u8) -> u8 {
        let value = self.bytes[register as usize];
        if register == Register::StatusC as u8 {
            self.c_reads += 1;
            self.bytes[register as usize] = 0;
        }
        if register == Register::StatusA as u8 {
            self.a_reads += 1;
            if self.late_uip && self.a_reads == 2 {
                return value | UIP;
            }
        }
        if register == Register::Second as u8 && self.change_second {
            self.bytes[register as usize] = value + 1;
        }
        value
    }
    fn write_register(&mut self, register: u8, value: u8) {
        self.writes.push((register, value));
        self.bytes[register as usize] = value;
    }
    fn century_register(&self) -> Option<NonZeroU8> {
        None
    }
    fn divider_mode(&self) -> DividerMode {
        DividerMode::Standard
    }
}

fn time() -> RtcAlarmTime {
    RtcAlarmTime {
        hour: Some(23),
        minute: Some(58),
        second: Some(47),
    }
}

#[ktest]
fn alarm_known_encodings_and_control_preservation() {
    for (mode, expected) in [
        (0, [0x91, 0x58, 0x47]),
        (2, [0x23, 0x58, 0x47]),
        (4, [0x8b, 58, 47]),
        (6, [23, 58, 47]),
    ] {
        let mut memory = Memory::new(mode | 0x50);
        memory.bytes[Register::StatusC as usize] = IRQF | AF;
        assert_eq!(
            write(&mut memory, time(), RtcAlarmState::Enabled),
            Ok(Some(()))
        );
        assert_eq!(raw(&mut memory), expected);
        assert_eq!(memory.bytes[Register::StatusB as usize], mode | 0x50 | AIE);
        let alarm = read(&mut memory, true).unwrap().unwrap();
        assert_eq!(alarm.time, time());
        assert_eq!(alarm.state, RtcAlarmState::Enabled);
        assert!(alarm.pending);
        assert_eq!(memory.c_reads, 2); // read_alarm must not consume Status C.
    }
}

#[ktest]
fn alarm_midnight_noon_and_wildcard_fields() {
    for (mode, midnight, noon) in [(0, 0x12, 0x92), (2, 0, 0x12), (4, 12, 0x8c), (6, 0, 12)] {
        let mut memory = Memory::new(mode);
        for (hour, expected) in [(0, midnight), (12, noon)] {
            let time = RtcAlarmTime {
                hour: Some(hour),
                minute: None,
                second: None,
            };
            assert_eq!(
                write(&mut memory, time, RtcAlarmState::Disabled),
                Ok(Some(()))
            );
            assert_eq!(raw(&mut memory), [expected, 0xff, 0xff]);
            assert_eq!(read(&mut memory, false).unwrap().unwrap().time, time);
        }
        for wildcard in [0xc0, 0xd5, 0xff] {
            for register in REGISTERS {
                memory.bytes[register as usize] = wildcard;
            }
            assert_eq!(
                read(&mut memory, false).unwrap().unwrap().time,
                RtcAlarmTime {
                    hour: None,
                    minute: None,
                    second: None
                }
            );
        }
    }
}

#[ktest]
fn alarm_invalid_input_never_writes() {
    let mut memory = Memory::new(2);
    for invalid in [
        RtcAlarmTime {
            hour: Some(24),
            ..time()
        },
        RtcAlarmTime {
            minute: Some(60),
            ..time()
        },
        RtcAlarmTime {
            second: Some(60),
            ..time()
        },
    ] {
        assert_eq!(
            write(&mut memory, invalid, RtcAlarmState::Enabled),
            Err(RtcError::InvalidTime)
        );
        assert!(memory.writes.is_empty());
    }
}

#[ktest]
fn alarm_rejects_malformed_registers_before_arming() {
    for (mode, hour, minute, second) in [
        (0, 0, 0, 0),
        (0, 0x13, 0, 0),
        (2, 0x24, 0, 0),
        (2, 0, 0x6a, 0),
        (6, 0, 0, 60),
    ] {
        let mut memory = Memory::new(mode);
        for (register, value) in REGISTERS.into_iter().zip([hour, minute, second]) {
            memory.bytes[register as usize] = value;
        }
        assert_eq!(read(&mut memory, false), Err(RtcError::InvalidTime));
        assert_eq!(
            set_state(&mut memory, RtcAlarmState::Enabled),
            Err(RtcError::InvalidTime)
        );
        assert!(memory.writes.is_empty());
    }
}

#[ktest]
fn alarm_retries_uip_set_and_overlapping_updates() {
    for (register, value) in [(Register::StatusA, UIP), (Register::StatusB, SET | 2)] {
        let mut memory = Memory::new(2);
        memory.bytes[register as usize] = value;
        assert_eq!(read(&mut memory, false), Ok(None));
        assert_eq!(write(&mut memory, time(), RtcAlarmState::Enabled), Ok(None));
        assert!(memory.writes.is_empty());
        // Cancellation stays available even if the calendar is inaccessible.
        assert_eq!(
            set_state(&mut memory, RtcAlarmState::Disabled),
            Ok(Some(()))
        );
    }
    for late_uip in [true, false] {
        let mut memory = Memory::new(2 | AIE);
        memory.late_uip = late_uip;
        memory.change_second = !late_uip;
        assert_eq!(write(&mut memory, time(), RtcAlarmState::Enabled), Ok(None));
        assert_eq!(memory.bytes[Register::StatusB as usize] & AIE, 0);
    }
}

#[ktest]
fn alarm_acknowledges_sources_and_delivers_once() {
    for (status_c, enabled, delivered) in [
        (IRQF | AF, true, true),
        (AF, true, false),
        (IRQF | 0x40, true, false),
        (IRQF | AF, false, false),
        (0, true, false),
    ] {
        let mut memory = Memory::new(2 | 0x50 | if enabled { AIE } else { 0 });
        memory.bytes[Register::StatusC as usize] = status_c;
        assert_eq!(acknowledge(&mut memory), delivered);
        assert_eq!(memory.bytes[Register::StatusC as usize], 0);
        assert_eq!(memory.bytes[Register::StatusB as usize] & !AIE, 2 | 0x50);
        assert!(!acknowledge(&mut memory));
        memory.bytes[Register::StatusC as usize] = IRQF | AF;
        if delivered {
            assert!(!acknowledge(&mut memory));
        }
    }
}

#[ktest]
fn alarm_rearming_flushes_stale_status_c() {
    let mut memory = Memory::new(2);
    assert_eq!(
        write(&mut memory, time(), RtcAlarmState::Disabled),
        Ok(Some(()))
    );
    memory.bytes[Register::StatusC as usize] = IRQF | AF;
    assert_eq!(set_state(&mut memory, RtcAlarmState::Enabled), Ok(Some(())));
    assert!(!acknowledge(&mut memory));
    assert_eq!(
        set_state(&mut memory, RtcAlarmState::Disabled),
        Ok(Some(()))
    );
    assert_eq!(memory.bytes[Register::StatusB as usize], 2);
}
