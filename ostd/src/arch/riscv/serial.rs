// SPDX-License-Identifier: MPL-2.0

//! The console I/O.

use core::fmt;

use spin::Once;

use crate::sync::{LocalIrqDisabled, SpinLock};

/// The primary serial port, which serves as an early console.
pub(crate) static SERIAL_PORT: Once<SpinLock<SbiSerial, LocalIrqDisabled>> =
    Once::initialized(SpinLock::new(SbiSerial::new()));

/// A serial port that is implemented via RISC-V SBI.
pub(crate) struct SbiSerial {
    _private: (),
}

impl SbiSerial {
    const fn new() -> Self {
        Self { _private: () }
    }
}

impl fmt::Write for SbiSerial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.as_bytes() {
            sbi_rt::console_write_byte(*c);
        }
        Ok(())
    }
}

/// Initializes the serial port.
pub(crate) fn init() {}
