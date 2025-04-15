// SPDX-License-Identifier: MPL-2.0

//! The console I/O.

use alloc::fmt;
use core::fmt::Write;

/// Prints the formatted arguments to the standard output using the serial port.
#[inline]
pub fn print(args: fmt::Arguments) {
    Stdout.write_fmt(args).unwrap();
}

struct Stdout;

impl Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &c in s.as_bytes() {
            send(c);
        }
        Ok(())
    }
}

/// Initializes the serial port.
pub(crate) fn init() {}

/// Sends a byte on the serial port.
pub fn send(data: u8) {
    sbi_rt::console_write_byte(data);
}
