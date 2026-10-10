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

use alloc::sync::Arc;
use core::num::NonZeroU8;

use ostd::{
    arch::{
        cpu::cpuid::cpuid,
        device::io_port::{ReadWriteAccess, WriteOnlyAccess},
        irq::{IRQ_CHIP, MappedIrqLine},
        kernel::ACPI_INFO,
        read_tsc, tsc_freq,
    },
    io::IoPort,
    irq::IrqLine,
    sync::{LocalIrqDisabled, SpinLock},
    warn,
};

use super::{Driver, RtcAlarm, RtcAlarmCallback, RtcAlarmState, RtcAlarmTime, RtcError};
use crate::SystemTime;

mod alarm;
#[cfg(ktest)]
mod alarm_test;
mod calendar;
#[cfg(ktest)]
mod time_test;

use calendar::{Access, DividerMode, Register};

pub(super) struct RtcCmos {
    // Keep the mapping alive; unmap before releasing the port owner on drop.
    alarm_irq: Option<MappedIrqLine>,
    // Time, alarm configuration, IRQ acknowledgement and event state share one lock.
    access: Arc<SpinLock<CmosAccess, LocalIrqDisabled>>,
}

impl Driver for RtcCmos {
    fn try_new() -> Option<Self> {
        // Port 0x70 also controls NMI masking. OSTD currently does not support NMIs;
        // this must be revisited when NMI support is added.
        const IOPORT_SEL: u16 = 0x70;
        const IOPORT_VAL: u16 = 0x71;

        let acpi_info = ACPI_INFO.get()?;
        if acpi_info
            .boot_flags
            .is_some_and(|flags| flags.use_time_and_alarm_namespace_for_rtc())
        {
            return None;
        }

        let (io_sel, io_val) = match (IoPort::acquire(IOPORT_SEL), IoPort::acquire(IOPORT_VAL)) {
            (Ok(io_sel), Ok(io_val)) => (io_sel, io_val),
            _ => {
                warn!("Failed to acquire CMOS RTC PIO region");
                return None;
            }
        };

        let divider_mode = cpuid(0, 0)
            .map(|id| {
                let mut vendor = [0; 12];
                vendor[..4].copy_from_slice(&id.ebx.to_le_bytes());
                vendor[4..8].copy_from_slice(&id.edx.to_le_bytes());
                vendor[8..].copy_from_slice(&id.ecx.to_le_bytes());
                if &vendor == b"AuthenticAMD" || &vendor == b"HygonGenuine" {
                    DividerMode::Amd
                } else {
                    DividerMode::Standard
                }
            })
            .unwrap_or(DividerMode::Standard);
        let mut access = CmosAccess {
            io_sel,
            io_val,
            century_register: acpi_info.century_register,
            divider_mode,
            alarm_pending: false,
            alarm_callback: None,
        };

        if access.read_register(Register::StatusD as u8) != calendar::VRT {
            warn!("CMOS RTC reports invalid RAM/time status, ignoring this device");
            return None;
        }

        // Take ownership of legacy interrupt sources before routing IRQ 8.
        let status_b = access.read_register(Register::StatusB as u8);
        access.write_register(Register::StatusB as u8, status_b & !0x70);
        access.read_register(Register::StatusC as u8);
        let access = Arc::new(SpinLock::new(access));
        let alarm_irq = Self::route_alarm_irq(&access);
        if alarm_irq.is_none() {
            warn!("CMOS RTC IRQ unavailable; alarm operations disabled");
        }
        Some(Self { alarm_irq, access })
    }

    fn read_rtc(&self) -> Result<SystemTime, RtcError> {
        self.retry(|| calendar::read(&mut *self.access.lock()))
    }

    fn set_rtc(&self, time: &SystemTime) -> Result<(), RtcError> {
        self.retry(|| calendar::write(&mut *self.access.lock(), time))
    }

    fn read_alarm(&self) -> Result<RtcAlarm, RtcError> {
        self.require_alarm_irq()?;
        self.retry(|| {
            let mut access = self.access.lock();
            let pending = access.alarm_pending;
            alarm::read(&mut *access, pending)
        })
    }

    fn set_alarm(&self, time: RtcAlarmTime, state: RtcAlarmState) -> Result<(), RtcError> {
        self.require_alarm_irq()?;
        self.retry(|| {
            let mut access = self.access.lock();
            let result = alarm::write(&mut *access, time, state)?;
            if result.is_some() {
                access.alarm_pending = false;
            }
            Ok(result)
        })
    }

    fn set_alarm_state(&self, state: RtcAlarmState) -> Result<(), RtcError> {
        self.require_alarm_irq()?;
        self.retry(|| {
            let mut access = self.access.lock();
            let result = alarm::set_state(&mut *access, state)?;
            if result.is_some() && state == RtcAlarmState::Enabled {
                access.alarm_pending = false;
            }
            Ok(result)
        })
    }

    fn set_alarm_callback(&self, callback: Option<Arc<RtcAlarmCallback>>) -> Result<(), RtcError> {
        self.require_alarm_irq()?;
        // Captured values may have destructors that access the RTC.
        let old = core::mem::replace(&mut self.access.lock().alarm_callback, callback);
        drop(old);
        Ok(())
    }

    fn take_alarm_event(&self) -> Result<bool, RtcError> {
        self.require_alarm_irq()?;
        Ok(core::mem::take(&mut self.access.lock().alarm_pending))
    }
}

impl RtcCmos {
    fn require_alarm_irq(&self) -> Result<(), RtcError> {
        self.alarm_irq
            .as_ref()
            .map(|_| ())
            .ok_or(RtcError::Unsupported)
    }

    fn route_alarm_irq(
        access: &Arc<SpinLock<CmosAccess, LocalIrqDisabled>>,
    ) -> Option<MappedIrqLine> {
        let chip = IRQ_CHIP.get()?;
        if chip.count_io_apics() == 0 {
            return None;
        }
        let mut line = IrqLine::alloc().ok()?;
        let weak = Arc::downgrade(access);
        line.on_active(move |_| {
            let Some(access) = weak.upgrade() else { return };
            let callback = {
                let mut access = access.lock();
                if alarm::acknowledge(&mut *access) {
                    access.alarm_pending = true;
                    access.alarm_callback.clone()
                } else {
                    None
                }
            };
            if let Some(callback) = callback {
                callback();
            }
        });
        chip.map_isa_pin_to(line, 8).ok()
    }

    fn retry<T>(
        &self,
        operation: impl FnMut() -> Result<Option<T>, RtcError>,
    ) -> Result<T, RtcError> {
        // OSTD initializes the TSC frequency before component initialization.
        // Use it directly so RTC access does not depend on RTC-based time calibration.
        const TIMEOUT_MILLIS: u64 = 100;
        let timeout_cycles = (tsc_freq() / 1000 * TIMEOUT_MILLIS).max(1);
        retry_until(timeout_cycles, operation, read_tsc)
    }
}

fn retry_until<T>(
    timeout_cycles: u64,
    mut operation: impl FnMut() -> Result<Option<T>, RtcError>,
    mut read_cycles: impl FnMut() -> u64,
) -> Result<T, RtcError> {
    let start = read_cycles();
    loop {
        if let Some(value) = operation()? {
            return Ok(value);
        }
        if read_cycles().wrapping_sub(start) >= timeout_cycles {
            return Err(RtcError::Timeout);
        }
        // Each attempt releases the CMOS lock and restores local interrupts before retrying.
        core::hint::spin_loop();
    }
}

struct CmosAccess {
    io_sel: IoPort<u8, WriteOnlyAccess>,
    io_val: IoPort<u8, ReadWriteAccess>,
    century_register: Option<NonZeroU8>,
    divider_mode: DividerMode,
    alarm_pending: bool,
    alarm_callback: Option<Arc<RtcAlarmCallback>>,
}

impl Access for CmosAccess {
    fn read_register(&mut self, register: u8) -> u8 {
        self.io_sel.write(register);
        self.io_val.read()
    }

    fn write_register(&mut self, register: u8, value: u8) {
        self.io_sel.write(register);
        self.io_val.write(value);
    }

    fn century_register(&self) -> Option<NonZeroU8> {
        self.century_register
    }

    fn divider_mode(&self) -> DividerMode {
        self.divider_mode
    }
}
