// SPDX-License-Identifier: MPL-2.0

use fdt_util::AcquireIoMems;
use ostd::{arch::boot::DEVICE_TREE, io::IoMem, mm::VmIoOnce};

use crate::{SystemTime, rtc::Driver};

pub struct RtcLoongson {
    io_mem: IoMem,
}

impl Driver for RtcLoongson {
    fn try_new() -> Option<Self> {
        const FDT_COMPATIBLE: &str = "loongson,ls7a-rtc";

        let [io_mem] = DEVICE_TREE
            .get()
            .unwrap()
            .find_compatible(&[FDT_COMPATIBLE])?
            .acquire_io_mems([SYS_MAX])?;

        // Initialize the RTC unit
        // Reference: <https://loongson.github.io/LoongArch-Documentation/Loongson-7A1000-usermanual-EN.html#rtc>
        io_mem.write_once(SYS_TOYTRIM, &0x0u32).unwrap();
        io_mem.write_once(SYS_RTCCTRL, &0x2900u32).unwrap();

        Some(Self { io_mem })
    }

    fn read_rtc(&self) -> SystemTime {
        // Read the Time of Year (TOY) counter and the RTC timer counter
        // Reference: <https://loongson.github.io/LoongArch-Documentation/Loongson-7A1000-usermanual-EN.html#rtc>
        let sys_toyread0: u32 = self.io_mem.read_once(SYS_TOYREAD0).unwrap();
        let sys_toyread1: u32 = self.io_mem.read_once(SYS_TOYREAD1).unwrap();
        let sys_rtcread: u32 = self.io_mem.read_once(SYS_RTCREAD).unwrap();

        let toy_year = sys_toyread1 as u16 + 1900;
        let toy_month = ((sys_toyread0 >> 26) & 0x3f) as u8;
        let toy_day = ((sys_toyread0 >> 21) & 0x1f) as u8;
        let toy_hour = ((sys_toyread0 >> 16) & 0x1f) as u8;
        let toy_minute = ((sys_toyread0 >> 10) & 0x3f) as u8;
        let toy_second = ((sys_toyread0 >> 4) & 0x3f) as u8;
        let nanos = sys_rtcread as u64 % 32768 * 1_000_000_000 / 32768;

        SystemTime {
            year: toy_year,
            month: toy_month,
            day: toy_day,
            hour: toy_hour,
            minute: toy_minute,
            second: toy_second,
            nanos,
        }
    }
}

const SYS_TOYTRIM: usize = 0x20;
const SYS_TOYREAD0: usize = 0x2c;
const SYS_TOYREAD1: usize = 0x30;

const SYS_RTCCTRL: usize = 0x40;
const SYS_RTCREAD: usize = 0x68;

const SYS_MAX: usize = 0x78;
