// SPDX-License-Identifier: MPL-2.0

use ostd::{arch::boot::DEVICE_TREE, io::IoMem, mm::VmIoOnce};

use crate::{rtc::Driver, SystemTime};

pub struct RtcLoongson {
    io_mem: IoMem,
}

impl Driver for RtcLoongson {
    fn try_new() -> Option<Self> {
        let chosen = DEVICE_TREE.get().unwrap().find_node("/rtc").unwrap();
        if let Some(compatible) = chosen.compatible()
            && compatible.all().any(|c| c == "loongson,ls7a-rtc")
        {
            let region = chosen.reg().unwrap().next().unwrap();
            let io_mem = IoMem::acquire(
                region.starting_address as usize
                    ..region.starting_address as usize + region.size.unwrap(),
            )
            .unwrap();

            const SYS_RTCCTRL: usize = 0x40;
            const SYS_TOYTRIM: usize = 0x20;
            // Initialize the RTC unit
            // Reference: <https://loongson.github.io/LoongArch-Documentation/Loongson-7A1000-usermanual-EN.html#rtc>
            io_mem.write_once(SYS_TOYTRIM, &0x0u32);
            io_mem.write_once(SYS_RTCCTRL, &0x2900u32);

            Some(Self { io_mem })
        } else {
            None
        }
    }

    fn read_rtc(&self) -> SystemTime {
        const SYS_TOYREAD0: usize = 0x2c;
        const SYS_TOYREAD1: usize = 0x30;
        const SYS_RTCREAD: usize = 0x68;

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
