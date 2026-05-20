// SPDX-License-Identifier: MPL-2.0

//! The console I/O via ARM PL011 UART.

use core::fmt;

use spin::Once;

use crate::sync::{LocalIrqDisabled, SpinLock};

/// The primary serial port, which serves as an early console.
pub static SERIAL_PORT: Once<SpinLock<Pl011Serial, LocalIrqDisabled>> = Once::new();

/// A serial port implemented via ARM PL011 UART.
pub struct Pl011Serial {
    _private: (),
}

impl fmt::Write for Pl011Serial {
    fn write_str(&mut self, _s: &str) -> fmt::Result {
        Ok(())
    }
}

/// Initializes the serial port.
pub(crate) fn init() {
    // TODO
}
