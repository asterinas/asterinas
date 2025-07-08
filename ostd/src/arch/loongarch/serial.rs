// SPDX-License-Identifier: MPL-2.0

//! The console I/O.

use spin::mutex::SpinMutex;

use super::device::serial::Serial;

/// The UART address.
// FIXME: Acquire it from device tree file. Because output is needed before device tree file is resolved,
// so we directly use a fixed address.
const UART_ADDR: usize = 0x800000001FE001E0;

/// The console UART.
///
/// SAFETY: It is safe because the UART address is acquired from the device tree,
/// and be mapped in DMW2.
static CONSOLE_COM1: SpinMutex<Serial> = SpinMutex::new(unsafe { Serial::new(UART_ADDR) });

/// Initializes the serial port.
pub(crate) fn init() {}

/// Sends a byte on the serial port.
pub(crate) fn send(data: u8) {
    let mut uart = CONSOLE_COM1.lock();
    match data {
        b'\n' => {
            uart.send(b'\r');
            uart.send(b'\n');
        }
        c => uart.send(c),
    }
}
