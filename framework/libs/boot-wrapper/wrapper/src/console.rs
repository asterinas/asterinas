use core::fmt::{self, Write};

use uart_16550::SerialPort;

struct Stdout {
    serial_port: SerialPort,
}

static mut STDOUT: Stdout = Stdout {
    serial_port: unsafe { SerialPort::new(0x0) },
};

/// safety: this function must only be called once
pub unsafe fn init() {
    STDOUT = Stdout::init();
}

impl Stdout {
    /// safety: this function must only be called once
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

/// This is used when dyn Trait is not supported or fmt::Arguments is fragile to use in PIE.
///
/// Safety: init() must be called before print_str() and there should be no race conditions.
pub unsafe fn print_str(s: &str) {
    STDOUT.write_str(s).unwrap();
}

unsafe fn print_char(c: char) {
    STDOUT.serial_port.send(c as u8);
}

/// This is used when dyn Trait is not supported or fmt::Arguments is fragile to use in PIE.
///
/// Safety: init() must be called before print_hex() and there should be no race conditions.
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
/// Safety: init() must be called before print_fmt() and there should be no race conditions.
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
