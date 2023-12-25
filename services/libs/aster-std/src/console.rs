//! `print` and `println` macros
//!
//! FIXME: It will print to all `virtio-console` devices, which is not a good choice.
//!

use core::fmt::{Arguments, Write};

struct VirtioConsolesPrinter;

impl Write for VirtioConsolesPrinter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for (_, device) in aster_console::all_devices() {
            device.send(s.as_bytes());
        }
        Ok(())
    }
}

pub fn _print(args: Arguments) {
    VirtioConsolesPrinter.write_fmt(args).unwrap();
}

/// Copy from Rust std: https://github.com/rust-lang/rust/blob/master/library/std/src/macros.rs
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        $crate::console::_print(format_args!($($arg)*));
    }};
}

/// Copy from Rust std: https://github.com/rust-lang/rust/blob/master/library/std/src/macros.rs
#[macro_export]
macro_rules! println {
    () => {
        $crate::print!("\n")
    };
    ($($arg:tt)*) => {{
        $crate::console::_print(format_args_nl!($($arg)*));
    }};
}
