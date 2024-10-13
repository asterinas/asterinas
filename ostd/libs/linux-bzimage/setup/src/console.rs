// SPDX-License-Identifier: MPL-2.0

use core::fmt::{self, Write};

use uart_16550::SerialPort;

struct Stdout {
    serial_port: SerialPort,
}

static mut STDOUT: Stdout = Stdout {
    serial_port: unsafe { SerialPort::new(0x0) },
};

/// SAFETY: this function must only be called once
pub unsafe fn init() {
    STDOUT = Stdout::init();
}

impl Stdout {
    /// SAFETY: this function must only be called once
    pub unsafe fn init() -> Self {
        let mut serial_port = unsafe { SerialPort::new(0x3F8) };
        serial_port.init();
        Self { serial_port }
    }
}

impl Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.serial_port.write_str(s).unwrap();
        Ok(())
    }
}

/// Print a string to the console.
///
/// This is used when dyn Trait is not supported or fmt::Arguments is fragile to use in PIE.
///
/// # Safety
///
/// [`init()`] must be called before it and there should be no race conditions.
pub unsafe fn print_str(s: &str) {
    #[allow(static_mut_refs)]
    STDOUT.write_str(s).unwrap();
}

/// Print a single character to the console.
///
/// This is used when dyn Trait is not supported or fmt::Arguments is fragile to use in PIE.
///
/// # Safety
///
/// [`init()`] must be called before it and there should be no race conditions.
unsafe fn print_char(c: char) {
    #[allow(static_mut_refs)]
    STDOUT.serial_port.send(c as u8);
}

/// Print a hexadecimal number to the console.
///
/// This is used when dyn Trait is not supported or fmt::Arguments is fragile to use in PIE.
///
/// # Safety
///
/// [`init()`] must be called before it and there should be no race conditions.
pub unsafe fn print_hex(n: u64) {
    print_str("0x");
    for i in (0..16).rev() {
        let digit = (n >> (i * 4)) & 0xf;
        if digit < 10 {
            print_char((b'0' + digit as u8) as char);
        } else {
            print_char((b'A' + (digit - 10) as u8) as char);
        }
    }
}

// TODO: Figure out why fmt::Arguments wont work even if relocations are applied.
// We just settle on simple print functions for now.
/*--------------------------------------------------------------------------------------------------

/// Glue code for print!() and println!() macros.
///
/// SAFETY: init() must be called before print_fmt() and there should be no race conditions.
pub unsafe fn print_fmt(args: fmt::Arguments) {
    STDOUT.write_fmt(args).unwrap();
}

#[macro_export]
macro_rules! print {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        unsafe {
            $crate::console::print_fmt(format_args!($fmt $(, $($arg)+)?))
        }
    }
}

#[macro_export]
macro_rules! println {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        unsafe {
            $crate::console::print_fmt(format_args!(concat!($fmt, "\n") $(, $($arg)+)?))
        }
    }
}

 *------------------------------------------------------------------------------------------------*/
