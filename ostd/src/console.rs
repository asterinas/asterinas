// SPDX-License-Identifier: MPL-2.0

//! Console output.

use core::fmt::{self, Arguments, Write};

use crate::sync::{LocalIrqDisabled, SpinLock};

struct Stdout;

impl Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &c in s.as_bytes() {
            crate::arch::serial::send(c);
        }
        Ok(())
    }
}

static STDOUT: SpinLock<Stdout, LocalIrqDisabled> = SpinLock::new(Stdout);

/// Prints formatted arguments to the console.
pub fn early_print(args: Arguments) {
    #[cfg(target_arch = "x86_64")]
    crate::arch::if_tdx_enabled!({
        // Hold the lock to prevent the logs from interleaving.
        let _guard = STDOUT.lock();
        tdx_guest::print(args);
    } else {
        STDOUT.lock().write_fmt(args).unwrap();
    });
    #[cfg(not(target_arch = "x86_64"))]
    STDOUT.lock().write_fmt(args).unwrap();
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
