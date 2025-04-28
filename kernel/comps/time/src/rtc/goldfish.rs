// SPDX-License-Identifier: MPL-2.0

use ostd::{arch::boot::DEVICE_TREE, io::IoMem, mm::VmIoOnce};
use chrono::{DateTime, Datelike, Timelike};

use crate::{SystemTime, rtc::Driver};

pub struct RtcGoldfish {
    io_mem: IoMem,
}

impl Driver for RtcGoldfish {
    fn try_new() -> Option<RtcGoldfish> {
        let chosen = DEVICE_TREE.get().unwrap().find_node("/soc/rtc").unwrap();
        if let Some(compatible) = chosen.compatible()
            && compatible.all().any(|c| c == "google,goldfish-rtc")
        {
            let region = chosen.reg().unwrap().next().unwrap();
            let io_mem = IoMem::acquire(
                region.starting_address as usize
                    ..region.starting_address as usize + region.size.unwrap(),
            )
            .unwrap();
            Some(RtcGoldfish { io_mem })
        } else {
            None
        }
    }

    fn read_rtc(&self) -> SystemTime {
        const TIME_LOW: usize = 0;
        const TIME_HIGH: usize = 4;

        let mut last_time_high = self.io_mem.read_once(TIME_HIGH).unwrap();
        let timestamp = loop {
            let time_low: u32 = self.io_mem.read_once(TIME_LOW).unwrap();
            let time_high: u32 = self.io_mem.read_once(TIME_HIGH).unwrap();
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
