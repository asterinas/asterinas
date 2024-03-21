// SPDX-License-Identifier: MPL-2.0

//! A simple debug console implementation based on legacy SBI calls.

use core::fmt::{self, Write};

use super::sbi;

struct Stdout;

impl Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            sbi::legacy::console_putchar(c);
        }
        Ok(())
    }
}

pub fn print(args: fmt::Arguments) {
    Stdout.write_fmt(args).unwrap();
}
