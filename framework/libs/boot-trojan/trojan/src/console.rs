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

pub fn print(args: fmt::Arguments) {
    // safety: init() must be called before print() and there is no race condition
    unsafe {
        STDOUT.write_fmt(args).unwrap();
    }
}

#[macro_export]
macro_rules! print {
  ($fmt: literal $(, $($arg: tt)+)?) => {
    $crate::console::print(format_args!($fmt $(, $($arg)+)?))
  }
}

#[macro_export]
macro_rules! println {
  ($fmt: literal $(, $($arg: tt)+)?) => {
    $crate::console::print(format_args!(concat!($fmt, "\n") $(, $($arg)+)?))
  }
}
