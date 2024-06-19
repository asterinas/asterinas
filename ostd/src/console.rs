// SPDX-License-Identifier: MPL-2.0

//! Console output.

use core::fmt::Arguments;

/// Prints formatted arguments to the console.
pub fn print(args: Arguments) {
    crate::arch::console::print(args);
}

/// Prints to the console.
#[macro_export]
macro_rules! early_print {
  ($fmt: literal $(, $($arg: tt)+)?) => {
    $crate::console::print(format_args!($fmt $(, $($arg)+)?))
  }
}

/// Prints to the console, with a newline.
#[macro_export]
macro_rules! early_println {
  () => { $crate::early_print!("\n") };
  ($fmt: literal $(, $($arg: tt)+)?) => {
    $crate::console::print(format_args!(concat!($fmt, "\n") $(, $($arg)+)?))
  }
}
