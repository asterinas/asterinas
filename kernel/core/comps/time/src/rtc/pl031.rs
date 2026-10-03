// SPDX-License-Identifier: MPL-2.0

//! PL031 RTC.
//!
//! This is a driver for ARM PrimeCell Real Time Clock (PL031).
//!
//! Reference: <https://developer.arm.com/documentation/ddi0224/b/Functional-Overview/ARM-PrimeCell-Real-Time-Clock--PL031--overview>

use chrono::DateTime;
use fdt_util::AcquireIoMems;
use ostd::{arch::boot::DEVICE_TREE, io::IoMem, mm::VmIoOnce};

use crate::{SystemTime, rtc::Driver};

pub(super) struct RtcPl031 {
    io_mem: IoMem,
}

impl Driver for RtcPl031 {
    fn try_new() -> Option<Self> {
        const FDT_COMPATIBLE: &str = "arm,pl031";

        let [io_mem] = DEVICE_TREE
            .get()
            .unwrap()
            .find_compatible(&[FDT_COMPATIBLE])?
            .acquire_io_mems([MAX_OFFSET])?;

        Some(Self { io_mem })
    }

    fn read_rtc(&self) -> SystemTime {
        let timestamp = self.io_mem.read_once::<u32>(RTCDR_OFFSET).unwrap();

        // This won't fail because the timestamp is a 32-bit integer.
        let time = DateTime::from_timestamp(timestamp as i64, 0).unwrap();
        SystemTime::from(time.naive_utc())
    }
}

const RTCDR_OFFSET: usize = 0;
const MAX_OFFSET: usize = 0x1000;
