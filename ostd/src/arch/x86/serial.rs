// SPDX-License-Identifier: MPL-2.0

//! The console I/O.

use super::device::serial::SerialPort;
use crate::io::reserve_io_port_range;

bitflags::bitflags! {
  struct LineSts: u8 {
    const INPUT_FULL = 1;
    const OUTPUT_EMPTY = 1 << 5;
  }
}

static CONSOLE_COM1_PORT: SerialPort = unsafe { SerialPort::new(0x3F8) };
reserve_io_port_range!(0x3F8..0x400);

/// Initializes the serial port.
pub(crate) fn init() {
    CONSOLE_COM1_PORT.init();
}

fn line_sts() -> LineSts {
    LineSts::from_bits_truncate(CONSOLE_COM1_PORT.line_status())
}

/// Sends a byte on the serial port.
pub(crate) fn send(data: u8) {
    match data {
        8 | 0x7F => {
            while !line_sts().contains(LineSts::OUTPUT_EMPTY) {}
            CONSOLE_COM1_PORT.send(8);
            while !line_sts().contains(LineSts::OUTPUT_EMPTY) {}
            CONSOLE_COM1_PORT.send(b' ');
            while !line_sts().contains(LineSts::OUTPUT_EMPTY) {}
            CONSOLE_COM1_PORT.send(8);
        }
        _ => {
            while !line_sts().contains(LineSts::OUTPUT_EMPTY) {}
            CONSOLE_COM1_PORT.send(data);
        }
    }
}
