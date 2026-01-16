// SPDX-License-Identifier: MPL-2.0

//! Console output.

use core::fmt::{Arguments, Write};

use crate::arch::serial::SERIAL_PORT;

pub mod uart_ns16650a;

/// Prints formatted arguments to the console.
pub fn early_print(args: Arguments) {
    let Some(serial) = SERIAL_PORT.get() else {
        return;
    };

    #[cfg(target_arch = "x86_64")]
    crate::arch::if_tdx_enabled!({
        // Hold the lock to prevent the logs from interleaving.
        let _guard = serial.lock();
        tdx_guest::print(args);
    } else {
        serial.lock().write_fmt(args).unwrap();
    });
    #[cfg(not(target_arch = "x86_64"))]
    serial.lock().write_fmt(args).unwrap();
}

/// Prints to the console.
#[macro_export]
macro_rules! early_print {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::console::early_print(format_args!($fmt $(, $($arg)+)?))
    }
}

/// Prints to the console with a newline.
#[macro_export]
macro_rules! early_println {
    () => { $crate::early_print!("\n") };
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::console::early_print(format_args!(concat!($fmt, "\n") $(, $($arg)+)?))
    }
}
