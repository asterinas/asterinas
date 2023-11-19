use core::fmt::Write;

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
    unsafe fn init() -> Self {
        let mut serial_port = unsafe { SerialPort::new(0x3F8) };
        serial_port.init();
        Self { serial_port }
    }
}

impl Stdout {
    fn write_str(&mut self, s: &str) {
        self.serial_port.write_str(s).unwrap();
    }

    fn write_char(&mut self, c: char) {
        self.serial_port.send(c as u8);
    }
}

/// Safety: init() must be called before print() and there should be no race condition
pub unsafe fn print(s: &str) {
    STDOUT.write_str(s);
}

/// Safety: init() must be called before print_char() and there should be no race condition
pub unsafe fn print_char(c: char) {
    STDOUT.write_char(c);
}

// Safety: init() must be called before print_hex() and there should be no race condition
pub unsafe fn print_hex(n: usize) {
    print("0x");
    let mut n = n;
    for _ in 0..16 {
        let digit = (n & 0xf) as u8;
        n >>= 4;
        let c = if digit < 10 {
            (b'0' + digit) as char
        } else {
            (b'a' + digit - 10) as char
        };
        print_char(c);
    }
}
