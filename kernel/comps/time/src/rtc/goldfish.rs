// SPDX-License-Identifier: MPL-2.0

use chrono::{DateTime, Datelike, Timelike};
use log::warn;
use ostd::{arch::boot::DEVICE_TREE, io::IoMem, mm::VmIoOnce};

use crate::{rtc::Driver, SystemTime};

pub struct RtcGoldfish {
    io_mem: IoMem,
}

impl Driver for RtcGoldfish {
    fn try_new() -> Option<Self> {
        const FDT_COMPATIBLE: &str = "google,goldfish-rtc";

        let node = DEVICE_TREE
            .get()
            .unwrap()
            .find_compatible(&[FDT_COMPATIBLE])?;

        let Some(mut reg) = node.reg() else {
            warn!("Goldfish node should have exactly one `reg` property, but found zero `reg`s");
            return None;
        };
        let Some(region) = reg.next() else {
            warn!("Goldfish node should have exactly one `reg` property, but found zero `reg`s");
            return None;
        };
        if reg.next().is_some() {
            warn!(
                "Goldfish node should have exactly one `reg` property, but found {} `reg`s",
                reg.count() + 2
            );
            return None;
        }

        let addr_start = region.starting_address as usize;
        let Some(addr_end) = addr_start.checked_add(region.size.unwrap()) else {
            warn!("Goldfish RTC register region size overflows");
            return None;
        };
        let Ok(io_mem) = IoMem::acquire(addr_start..addr_end) else {
            warn!("Failed to acquire Goldfish RTC MMIO region");
            return None;
        };

        Some(Self { io_mem })
    }

    fn read_rtc(&self) -> SystemTime {
        const LOWER_HALF_OFFSET: usize = 0;
        const HIGHER_HALF_OFFSET: usize = 4;

        let mut last_time_high = self.io_mem.read_once(HIGHER_HALF_OFFSET).unwrap();
        let timestamp = loop {
            let time_low: u32 = self.io_mem.read_once(LOWER_HALF_OFFSET).unwrap();
            let time_high: u32 = self.io_mem.read_once(HIGHER_HALF_OFFSET).unwrap();
            if last_time_high == time_high {
                break ((time_high as u64) << 32) | time_low as u64;
            }
            last_time_high = time_high;
        };

        let time = DateTime::from_timestamp_nanos(timestamp as i64).naive_utc();
        let (is_ad, year) = time.year_ce();
        debug_assert!(is_ad, "non-negative timestamp should always be AD");

        SystemTime {
            year: year as u16,
            month: time.month() as u8,
            day: time.day() as u8,
            hour: time.hour() as u8,
            minute: time.minute() as u8,
            second: time.second() as u8,
            nanos: time.nanosecond() as u64,
        }
    }
}
