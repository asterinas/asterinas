// SPDX-License-Identifier: MPL-2.0

//! The console I/O.

/// Initializes the serial port.
pub(crate) fn init() {}

/// Sends a byte on the serial port.
pub(crate) fn send(data: u8) {
    sbi_rt::console_write_byte(data);
}
