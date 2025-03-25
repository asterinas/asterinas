// SPDX-License-Identifier: MPL-2.0

//! Console output.

use core::fmt::Arguments;

/// Prints formatted arguments to the console.
pub fn early_print(args: Arguments) {
    #[cfg(target_arch = "x86_64")]
    crate::arch::if_tdx_enabled!({
        tdx_guest::print(args);
    } else {
        crate::arch::serial::print(args);
    });
    #[cfg(not(target_arch = "x86_64"))]
    crate::arch::serial::print(args);
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
