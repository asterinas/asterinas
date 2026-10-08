// SPDX-License-Identifier: MPL-2.0

//! Calendar behavior against register fixtures, including broken hardware responses.

use alloc::vec::Vec;

use ostd::prelude::ktest;

use super::*;

struct Memory {
    registers: [u8; 128],
    century: Option<NonZeroU8>,
    divider: DividerMode,
    writes: Vec<(u8, u8)>,
    status_a_reads: usize,
    uip_on_read: Option<usize>,
    change_second: Option<u8>,
    second_reads: usize,
}

impl Memory {
    fn new(status_b: u8) -> Self {
        let mut registers = [0x5a; 128];
        registers[Register::StatusA as usize] = 0x26;
        registers[Register::StatusB as usize] = status_b;
        registers[Register::StatusD as usize] = VRT;
        Self {
            registers,
            century: NonZeroU8::new(0x32),
            divider: DividerMode::Standard,
            writes: Vec::new(),
            status_a_reads: 0,
            uip_on_read: None,
            change_second: None,
            second_reads: 0,
        }
    }
}

impl Access for Memory {
    fn read_register(&mut self, register: u8) -> u8 {
        if register == Register::StatusA as u8 {
            self.status_a_reads += 1;
            if self.uip_on_read == Some(self.status_a_reads) {
                return self.registers[register as usize] | UIP;
            }
        }
        if register == Register::Second as u8 {
            self.second_reads += 1;
            if self.second_reads == 3
                && let Some(second) = self.change_second
            {
                self.registers[register as usize] = second;
            }
        }
        self.registers[register as usize]
    }

    fn write_register(&mut self, register: u8, value: u8) {
        self.writes.push((register, value));
        self.registers[register as usize] = value;
    }

    fn century_register(&self) -> Option<NonZeroU8> {
        self.century
    }

    fn divider_mode(&self) -> DividerMode {
        self.divider
    }
}

fn sample() -> SystemTime {
    SystemTime {
        year: 2024,
        month: 2,
        day: 29,
        hour: 23,
        minute: 58,
        second: 47,
        nanos: 0,
    }
}

#[ktest]
fn reads_known_bcd_and_binary_registers() {
    // Fixtures come directly from the CMOS encoding, independently of write().
    for (status_b, raw, century) in [
        (HOUR_24, [0x24, 0x02, 0x29, 0x23, 0x58, 0x47], 0x20),
        (BINARY | HOUR_24, [24, 2, 29, 23, 58, 47], 20),
        (0, [0x24, 0x02, 0x29, 0x91, 0x58, 0x47], 0x20),
        (BINARY, [24, 2, 29, 0x8b, 58, 47], 20),
    ] {
        let mut memory = Memory::new(status_b);
        for (register, value) in CALENDAR_REGISTERS.into_iter().zip(raw) {
            memory.registers[register as usize] = value;
        }
        memory.registers[0x32] = century;
        assert_eq!(read(&mut memory), Ok(Some(sample())));
        assert!(memory.writes.is_empty());
    }
}

#[ktest]
fn writes_known_bcd_and_binary_registers() {
    for (status_b, expected, century) in [
        (HOUR_24, [0x24, 0x02, 0x29, 0x23, 0x58, 0x47], 0x20),
        (BINARY | HOUR_24, [24, 2, 29, 23, 58, 47], 20),
        (0, [0x24, 0x02, 0x29, 0x91, 0x58, 0x47], 0x20),
        (BINARY, [24, 2, 29, 0x8b, 58, 47], 20),
    ] {
        let mut memory = Memory::new(status_b);
        assert_eq!(write(&mut memory, &sample()), Ok(Some(())));
        for (register, value) in CALENDAR_REGISTERS.into_iter().zip(expected) {
            assert_eq!(memory.registers[register as usize], value);
        }
        assert_eq!(memory.registers[0x32], century);
    }
}

#[ktest]
fn midnight_and_noon_work_in_all_modes() {
    for status_b in [0, BINARY, HOUR_24, BINARY | HOUR_24] {
        for hour in [0, 1, 11, 12, 13, 23] {
            let mut memory = Memory::new(status_b);
            let time = SystemTime { hour, ..sample() };
            assert_eq!(write(&mut memory, &time), Ok(Some(())));
            if status_b & HOUR_24 == 0 && (hour == 0 || hour == 12) {
                let twelve = if status_b & BINARY == 0 { 0x12 } else { 12 };
                assert_eq!(
                    memory.registers[Register::Hour as usize],
                    twelve | if hour == 12 { PM } else { 0 }
                );
            }
            assert_eq!(read(&mut memory), Ok(Some(time)));
        }
    }
}

#[ktest]
fn invalid_dates_and_precision_do_not_write_registers() {
    for time in [
        SystemTime {
            year: 0,
            ..sample()
        },
        SystemTime {
            year: 10000,
            ..sample()
        },
        SystemTime {
            year: 2100,
            ..sample()
        },
        SystemTime {
            month: 0,
            ..sample()
        },
        SystemTime {
            month: 13,
            ..sample()
        },
        SystemTime { day: 0, ..sample() },
        SystemTime {
            day: 30,
            ..sample()
        },
        SystemTime {
            month: 4,
            day: 31,
            ..sample()
        },
        SystemTime {
            hour: 24,
            ..sample()
        },
        SystemTime {
            minute: 60,
            ..sample()
        },
        SystemTime {
            second: 60,
            ..sample()
        },
        SystemTime {
            nanos: 1,
            ..sample()
        },
        SystemTime {
            nanos: 1_000_000_000,
            ..sample()
        },
    ] {
        let mut memory = Memory::new(HOUR_24);
        let original = memory.registers;
        assert_eq!(write(&mut memory, &time), Err(RtcError::InvalidTime));
        assert_eq!(memory.registers, original);
        assert!(memory.writes.is_empty());
    }
}

#[ktest]
fn century_and_leap_year_boundaries() {
    for year in [1970, 1999, 2000, 2099, 2100, 2400, 9999] {
        let mut memory = Memory::new(HOUR_24);
        let time = SystemTime {
            year,
            month: 1,
            day: 1,
            ..sample()
        };
        assert_eq!(write(&mut memory, &time), Ok(Some(())));
        assert_eq!(read(&mut memory), Ok(Some(time)));
    }
    let mut memory = Memory::new(HOUR_24);
    assert_eq!(
        write(
            &mut memory,
            &SystemTime {
                year: 1969,
                month: 1,
                day: 1,
                ..sample()
            }
        ),
        Err(RtcError::InvalidTime)
    );
    assert!(memory.writes.is_empty());
    for (year, expected) in [(1900, false), (2000, true), (2100, false), (2400, true)] {
        let mut memory = Memory::new(HOUR_24);
        let time = SystemTime { year, ..sample() };
        assert_eq!(write(&mut memory, &time).is_ok(), expected);
    }
}

#[ktest]
fn absent_century_has_explicit_year_range() {
    for year in [1999, 2000, 2099, 2100] {
        let mut memory = Memory::new(HOUR_24);
        memory.century = None;
        let time = SystemTime {
            year,
            month: 1,
            day: 1,
            ..sample()
        };
        if (2000..=2099).contains(&year) {
            assert_eq!(write(&mut memory, &time), Ok(Some(())));
            assert_eq!(read(&mut memory), Ok(Some(time)));
        } else {
            assert_eq!(write(&mut memory, &time), Err(RtcError::InvalidTime));
            assert!(memory.writes.is_empty());
        }
    }
}

#[ktest]
fn rejects_invalid_hardware_encoding_and_calendar() {
    for (register, value) in [
        (Register::Minute, 0x1a),
        (Register::Second, 0x60),
        (Register::Month, 0x13),
        (Register::Day, 0x30),
        (Register::Hour, 0x24),
    ] {
        let mut memory = Memory::new(HOUR_24);
        write(&mut memory, &sample()).unwrap();
        memory.registers[register as usize] = value;
        assert_eq!(read(&mut memory), Err(RtcError::InvalidTime));
    }
    let mut memory = Memory::new(0);
    write(&mut memory, &sample()).unwrap();
    memory.registers[Register::Hour as usize] = 0;
    assert_eq!(read(&mut memory), Err(RtcError::InvalidTime));
}

#[ktest]
fn update_and_frozen_registers_require_retry() {
    for index in [1, 2] {
        let mut memory = Memory::new(HOUR_24);
        write(&mut memory, &sample()).unwrap();
        memory.status_a_reads = 0;
        memory.uip_on_read = Some(index);
        assert_eq!(read(&mut memory), Ok(None));
    }
    let mut memory = Memory::new(HOUR_24);
    write(&mut memory, &sample()).unwrap();
    memory.change_second = Some(0x48);
    assert_eq!(read(&mut memory), Ok(None));
    assert_eq!(read(&mut memory).unwrap().unwrap().second, 48);
    for (register, mask) in [(Register::StatusA, UIP), (Register::StatusB, SET)] {
        let mut memory = Memory::new(HOUR_24);
        memory.registers[register as usize] |= mask;
        assert_eq!(read(&mut memory), Ok(None));
        assert_eq!(write(&mut memory, &sample()), Ok(None));
        assert!(memory.writes.is_empty());
    }
}

#[ktest]
fn invalid_battery_status_is_reported() {
    let mut memory = Memory::new(HOUR_24);
    memory.registers[Register::StatusD as usize] = 0;
    assert_eq!(read(&mut memory), Err(RtcError::InvalidTime));
    assert_eq!(write(&mut memory, &sample()), Err(RtcError::InvalidTime));
    assert!(memory.writes.is_empty());
}

#[ktest]
fn restores_control_and_preserves_unrelated_registers() {
    for (divider, inhibited_a) in [(DividerMode::Standard, 0x76), (DividerMode::Amd, 0x26)] {
        let mut memory = Memory::new(0x7a);
        memory.divider = divider;
        memory.registers[Register::StatusA as usize] = 0x36;
        let original = memory.registers;
        write(&mut memory, &sample()).unwrap();
        assert_eq!(memory.writes[0], (Register::StatusB as u8, 0xfa));
        assert_eq!(memory.writes[1], (Register::StatusA as u8, inhibited_a));
        assert_eq!(
            memory.writes[memory.writes.len() - 2],
            (Register::StatusB as u8, 0x7a)
        );
        assert_eq!(memory.writes.last(), Some(&(Register::StatusA as u8, 0x36)));
        for (index, byte) in original.into_iter().enumerate() {
            if !CALENDAR_REGISTERS
                .iter()
                .any(|register| *register as usize == index)
                && index != 0x32
            {
                assert_eq!(memory.registers[index], byte);
            }
        }
    }
}
