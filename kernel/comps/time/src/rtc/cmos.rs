// SPDX-License-Identifier: MPL-2.0

//! CMOS RTC.
//!
//! "CMOS" is a tiny bit of very low power static memory that lives on the same chip as the
//! Real-Time Clock (RTC).
//!
//! According to the Linux implementation, in x86, if the CMOS/RTC is present at the legacy
//! addresses (I/O Ports 0x70 and 0x71), then it is an MC146818 CMOS/RTC. Therefore, in this
//! module, the register addresses and data interpretation are based on the MC146818 datasheet.
//!
//! Reference:
//! <https://elixir.bootlin.com/linux/v6.17.5/source/arch/x86/kernel/rtc.c#L69>
//! <https://www.scs.stanford.edu/23wi-cs212/pintos/specs/mc146818a.pdf>

use core::num::NonZeroU8;

use log::warn;
use ostd::{arch::{device::io_port::{ReadWriteAccess, WriteOnlyAccess}, kernel::ACPI_INFO}, io::IoPort, sync::SpinLock};

use crate::SystemTime;
use super::Driver;

pub struct RtcCmos {
    access: SpinLock<CmosAccess>,
    status_b: StatusB,
}

impl Driver for RtcCmos {
    fn try_new() -> Option<Self> {
        // TODO: Due to historical reasons, the "NMI Enable" bit (named `NMI_EN` in Intel's
        // datasheet) and the "Real Time Clock Index" bits are assigned to the same I/O port
        // (`IOPORT_SEL`). Currently, we do not support NMIs. However, once we add support, we
        // should reconsider the safety impact to allow OSTD users to safely manipulate the NMI
        // enablement.
        //
        // Reference:
        // <https://edc.intel.com/content/www/id/id/design/publications/core-ultra-p200s-series-processors-soc-i-o-registers/nmi-enable-and-real-time-clock-index-nmi-en-offset-70/>
        // <https://wiki.osdev.org/CMOS#Non-Maskable_Interrupts>
        const IOPORT_SEL: u16 = 0x70;
        const IOPORT_VAL: u16 = 0x71;

        let acpi_info = ACPI_INFO.get().unwrap();

        if acpi_info.boot_flags.is_some_and(|flags| flags.use_time_and_alarm_namespace_for_rtc()) {
            // "If set, indicates that the CMOS RTC is either not implemented, or does not exist at
            // the legacy addresses". See:
            // <https://uefi.org/specs/ACPI/6.5/05_ACPI_Software_Programming_Model.html#ia-pc-boot-architecture-flags>.
            return None;
        }

        let (io_sel, io_val) = match (IoPort::acquire(IOPORT_SEL), IoPort::acquire(IOPORT_VAL)) {
            (Ok(io_sel), Ok(io_val)) => (io_sel, io_val),
            _ => {
                warn!("Failed to acquire CMOS RTC PIO region");
                return None;
            }
        };

        let century_register = acpi_info.century_register;

        let mut access = CmosAccess {
            io_sel,
            io_val,
            century_register,
        };
        let status_b = access.read_status_b();

        // Ideally, the absence of the CMOS RTC should be reported in the ACPI tables (We've
        // checked the flag above). However, the ACPI tables are sometimes unreliable (e.g., QEMU
        // never sets the flag), so we need to perform additional checks.
        if !access.check_presence() {
            warn!("CMOS RTC reports unexpected status, ignoring this device");
            return None;
        }

        Some(Self {
            access: SpinLock::new(access),
            status_b,
        })
    }

    fn read_rtc(&self) -> SystemTime {
        CmosData::read_rtc(self).into()
    }
}

struct CmosAccess {
    io_sel: IoPort<u8, WriteOnlyAccess>,
    io_val: IoPort<u8, ReadWriteAccess>,
    century_register: Option<NonZeroU8>,
}

#[repr(u8)]
enum Register {
    Second = 0x00,
    Minute = 0x02,
    Hour = 0x04,
    Day = 0x07,
    Month = 0x08,
    Year = 0x09,

    StatusA = 0x0A,
    StatusB = 0x0B,
    // `StatusC` is not used.
    StatusD = 0x0D,
}

bitflags::bitflags! {
    struct StatusA: u8 {
        /// The update in progress (UIP) bit.
        const UIP = 1 << 7;
    }
}

bitflags::bitflags! {
    struct StatusB: u8 {
        /// The data mode (DM) bit.
        ///
        /// This bit is set when the binary format is used; otherwise, the BCD format is used.
        const DM_BINARY = 1 << 2;
        /// The clock mode (CM) bit.
        ///
        /// This bit is set when the 24-hour format is used; otherwise, the 12-hour format is used.
        const CM_24HOUR = 1 << 1;
    }
}

bitflags::bitflags! {
    struct StatusD: u8 {
        /// The valid RAM and time (VRT) bit.
        ///
        /// This bit is set when the RAM and time are valid.
        const VRT = 1 << 7;
    }
}

impl CmosAccess {
    pub(self) fn read_register(&mut self, reg: Register) -> u8 {
        self.read_register_impl(reg as u8)
    }

    pub(self) fn read_century(&mut self) -> Option<u8> {
        self.century_register.map(|r| self.read_register_impl(r.get()))
    }

    pub(self) fn read_status_a(&mut self) -> StatusA {
        StatusA::from_bits_truncate(self.read_register_impl(Register::StatusA as u8))
    }

    pub(self) fn read_status_b(&mut self) -> StatusB {
        StatusB::from_bits_truncate(self.read_register_impl(Register::StatusB as u8))
    }

    pub(self) fn check_presence(&mut self) -> bool {
        // If a working CMOS RTC is present, `VRT` should be set and all other reserved bits should
        // not be set.
        self.read_register_impl(Register::StatusD as u8) == StatusD::VRT.bits()
    }

    fn read_register_impl(&mut self, reg: u8) -> u8 {
        self.io_sel.write(reg);
        self.io_val.read()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CmosData {
    century: Option<NonZeroU8>,
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
}

impl CmosData {
    pub(self) fn read_rtc(rtc: &RtcCmos) -> Self {
        let mut access = rtc.access.lock();

        let mut now = Self::from_rtc_raw(&mut access);
        // Retry if the new value differs from the old one. An RTC update may occur in the
        // meantime, which would result in an invalid value.
        while let new = Self::from_rtc_raw(&mut access) && now != new {
            now = new;
        }

        drop(access);

        if !rtc.status_b.contains(StatusB::DM_BINARY) {
            now.convert_bcd_to_binary();
        }
        if !rtc.status_b.contains(StatusB::CM_24HOUR) {
            now.convert_12_hour_to_24_hour();
        }
        now.modify_year();

        now
    }

    fn from_rtc_raw(access: &mut CmosAccess) -> Self {
        // Wait if the RTC updates are in progress.
        while access.read_status_a().contains(StatusA::UIP) {
            core::hint::spin_loop();
        }

        let second = access.read_register(Register::Second);
        let minute = access.read_register(Register::Minute);
        let hour = access.read_register(Register::Hour);
        let day = access.read_register(Register::Day);
        let month = access.read_register(Register::Month);
        let year = access.read_register(Register::Year) as u16;
        let century = access.read_century().and_then(NonZeroU8::new);

        CmosData {
            century,
            year,
            month,
            day,
            hour,
            minute,
            second,
        }
    }

    /// Converts BCD values to binary values.
    fn convert_bcd_to_binary(&mut self) {
        fn bcd_to_binary(val: u8) -> u8 {
            (val & 0xF) + (val >> 4) * 10
        }

        self.second = bcd_to_binary(self.second);
        self.minute = bcd_to_binary(self.minute);
        self.hour = bcd_to_binary(self.hour & !Self::HOUR_IS_AFTERNOON) | (self.hour & Self::HOUR_IS_AFTERNOON);
        self.day = bcd_to_binary(self.day);
        self.month = bcd_to_binary(self.month);
        self.year = bcd_to_binary(self.year as u8) as u16;
        self.century = self.century.and_then(|c| NonZeroU8::new(bcd_to_binary(c.get())));
    }

    const HOUR_IS_AFTERNOON: u8 = 0x80;

    /// Converts the 12-hour clock to the 24-hour clock.
    fn convert_12_hour_to_24_hour(&mut self) {
        if self.hour & Self::HOUR_IS_AFTERNOON != 0 {
            self.hour = (self.hour & !Self::HOUR_IS_AFTERNOON) + 12;
        }
    }

    /// Converts the year without the century (e.g., 10) to the year with the century (e.g., 2010).
    fn modify_year(&mut self) {
        const DEFAULT_21_CENTURY: u8 = 20;

        self.year += (self.century.map(NonZeroU8::get).unwrap_or(DEFAULT_21_CENTURY) as u16) * 100;
    }
}

impl From<CmosData> for SystemTime {
    fn from(cmos: CmosData) -> SystemTime {
        SystemTime {
            year: cmos.year,
            month: cmos.month,
            day: cmos.day,
            hour: cmos.hour,
            minute: cmos.minute,
            second: cmos.second,
            nanos: 0,
        }
    }
}
