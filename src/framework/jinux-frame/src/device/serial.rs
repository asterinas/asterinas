use alloc::{sync::Arc, vec::Vec};
use lazy_static::lazy_static;
use spin::Mutex;

use crate::{cell::Cell, debug, driver::pic, x86_64_util::*, IrqAllocateHandle, TrapFrame};
use core::fmt::{self, Write};

bitflags::bitflags! {
  struct LineSts: u8 {
    const INPUT_FULL = 1;
    const OUTPUT_EMPTY = 1 << 5;
  }
}

/// A port-mapped UART. Copied from uart_16550.
const SERIAL_DATA: u16 = 0x3F8;
const SERIAL_INT_EN: u16 = SERIAL_DATA + 1;
const SERIAL_FIFO_CTRL: u16 = SERIAL_DATA + 2;
const SERIAL_LINE_CTRL: u16 = SERIAL_DATA + 3;
const SERIAL_MODEM_CTRL: u16 = SERIAL_DATA + 4;
const SERIAL_LINE_STS: u16 = SERIAL_DATA + 5;
lazy_static! {
    static ref CONSOLE_IRQ_CALLBACK: Cell<IrqAllocateHandle> = {
        let irq = Cell::new(pic::allocate_irq(4).unwrap());
        irq.get().on_active(handle_serial_input);
        irq
    };
    pub static ref SERIAL_INPUT_CALLBACKS: Mutex<Vec<Arc<dyn Fn(u8) + Send + Sync + 'static>>> =
        Mutex::new(Vec::new());
}

/// Initializes the serial port.
pub(crate) fn init() {
    // Disable interrupts
    out8(SERIAL_INT_EN, 0x00);
    // Enable DLAB
    out8(SERIAL_LINE_CTRL, 0x80);
    // Set maximum speed to 38400 bps by configuring DLL and DLM
    out8(SERIAL_DATA, 0x03);
    out8(SERIAL_INT_EN, 0x00);
    // Disable DLAB and set data word length to 8 bits
    out8(SERIAL_LINE_CTRL, 0x03);
    // Enable FIFO, clear TX/RX queues and
    // set interrupt watermark at 14 bytes
    out8(SERIAL_FIFO_CTRL, 0xC7);
    // Mark data terminal ready, signal request to send
    // and enable auxilliary output #2 (used as interrupt line for CPU)
    out8(SERIAL_MODEM_CTRL, 0x0B);
    // Enable interrupts
    out8(SERIAL_INT_EN, 0x01);
}

pub(crate) fn register_serial_input_irq_handler<F>(callback: F)
where
    F: Fn(&TrapFrame) + Sync + Send + 'static,
{
    CONSOLE_IRQ_CALLBACK.get().on_active(callback);
}

fn handle_serial_input(trap_frame: &TrapFrame) {
    // debug!("keyboard interrupt was met");
    if SERIAL_INPUT_CALLBACKS.is_locked() {
        return;
    }
    let lock = SERIAL_INPUT_CALLBACKS.lock();
    let received_char = receive_char().unwrap();
    debug!("receive char = {:?}", received_char);
    for callback in lock.iter() {
        callback(received_char);
    }
}

fn line_sts() -> LineSts {
    LineSts::from_bits_truncate(in8(SERIAL_LINE_STS))
}

/// Sends a byte on the serial port.
pub fn send(data: u8) {
    match data {
        8 | 0x7F => {
            while !line_sts().contains(LineSts::OUTPUT_EMPTY) {}
            out8(SERIAL_DATA, 8);
            while !line_sts().contains(LineSts::OUTPUT_EMPTY) {}
            out8(SERIAL_DATA, b' ');
            while !line_sts().contains(LineSts::OUTPUT_EMPTY) {}
            out8(SERIAL_DATA, 8)
        }
        _ => {
            while !line_sts().contains(LineSts::OUTPUT_EMPTY) {}
            out8(SERIAL_DATA, data);
        }
    }
}

/// Receives a byte on the serial port. non-blocking
pub fn receive_char() -> Option<u8> {
    if line_sts().contains(LineSts::INPUT_FULL) {
        Some(in8(SERIAL_DATA))
    } else {
        None
    }
}

struct Stdout;

impl Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &c in s.as_bytes() {
            send(c);
        }
        Ok(())
    }
}

pub fn print(args: fmt::Arguments) {
    Stdout.write_fmt(args).unwrap();
}

#[macro_export]
macro_rules! console_print {
  ($fmt: literal $(, $($arg: tt)+)?) => {
    $crate::device::serial::print(format_args!($fmt $(, $($arg)+)?))
  }
}

#[macro_export]
macro_rules! console_println {
  ($fmt: literal $(, $($arg: tt)+)?) => {
    $crate::device::serial::print(format_args!(concat!($fmt, "\n") $(, $($arg)+)?))
  }
}
