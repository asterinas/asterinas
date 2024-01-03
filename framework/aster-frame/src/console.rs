// SPDX-License-Identifier: MPL-2.0

use core::fmt::Arguments;

pub fn print(args: Arguments) {
    crate::arch::console::print(args);
}

#[macro_export]
macro_rules! early_print {
  ($fmt: literal $(, $($arg: tt)+)?) => {
    $crate::console::print(format_args!($fmt $(, $($arg)+)?))
  }
}

#[macro_export]
macro_rules! early_println {
  ($fmt: literal $(, $($arg: tt)+)?) => {
    $crate::console::print(format_args!(concat!($fmt, "\n") $(, $($arg)+)?))
  }
}
