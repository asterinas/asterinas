// SPDX-License-Identifier: MPL-2.0

use chrono::DateTime;
use fdt_util::AcquireIoMems;
use ostd::{arch::boot::DEVICE_TREE, io::IoMem, mm::VmIoOnce};

use crate::{SystemTime, rtc::Driver};

pub struct RtcGoldfish {
    io_mem: IoMem,
}

impl Driver for RtcGoldfish {
    fn try_new() -> Option<Self> {
        const FDT_COMPATIBLE: &str = "google,goldfish-rtc";

        let [io_mem] = DEVICE_TREE
            .get()
            .unwrap()
            .find_compatible(&[FDT_COMPATIBLE])?
            .acquire_io_mems([MAX_OFFSET])?;

        Some(Self { io_mem })
    }

    fn read_rtc(&self) -> SystemTime {
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
        SystemTime::from(time)
    }
}

const LOWER_HALF_OFFSET: usize = 0;
const HIGHER_HALF_OFFSET: usize = 4;
const MAX_OFFSET: usize = 8;
