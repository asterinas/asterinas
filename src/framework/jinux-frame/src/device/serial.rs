//! A port-mapped UART. Copied from uart_16550.

use super::io_port::{IoPort, ReadWriteAccess, WriteOnlyAccess};
use alloc::{sync::Arc, vec::Vec};
use core::fmt::{self, Write};
use log::debug;
use spin::{Mutex, Once};

use crate::{driver::pic_allocate_irq, trap::IrqAllocateHandle};
use trapframe::TrapFrame;

bitflags::bitflags! {
  struct LineSts: u8 {
    const INPUT_FULL = 1;
    const OUTPUT_EMPTY = 1 << 5;
  }
}

const SERIAL_DATA_PORT: u16 = 0x3F8;

static SERIAL_DATA: IoPort<u8, ReadWriteAccess> = unsafe { IoPort::new(SERIAL_DATA_PORT) };
static SERIAL_INT_EN: IoPort<u8, WriteOnlyAccess> = unsafe { IoPort::new(SERIAL_DATA_PORT + 1) };
static SERIAL_FIFO_CTRL: IoPort<u8, WriteOnlyAccess> = unsafe { IoPort::new(SERIAL_DATA_PORT + 2) };
static SERIAL_LINE_CTRL: IoPort<u8, WriteOnlyAccess> = unsafe { IoPort::new(SERIAL_DATA_PORT + 3) };
static SERIAL_MODEM_CTRL: IoPort<u8, WriteOnlyAccess> =
    unsafe { IoPort::new(SERIAL_DATA_PORT + 4) };
static SERIAL_LINE_STS: IoPort<u8, ReadWriteAccess> = unsafe { IoPort::new(SERIAL_DATA_PORT + 5) };

static CONSOLE_IRQ_CALLBACK: Once<Mutex<IrqAllocateHandle>> = Once::new();
static SERIAL_INPUT_CALLBACKS: Mutex<Vec<Arc<dyn Fn(u8) + Send + Sync + 'static>>> =
    Mutex::new(Vec::new());

/// Initializes the serial port.
pub(crate) fn init() {
    // Disable interrupts
    SERIAL_INT_EN.write(0x00);
    // Enable DLAB
    SERIAL_LINE_CTRL.write(0x80);
    // Set maximum speed to 38400 bps by configuring DLL and DLM
    SERIAL_DATA.write(0x03);
    SERIAL_INT_EN.write(0x00);
    // Disable DLAB and set data word length to 8 bits
    SERIAL_LINE_CTRL.write(0x03);
    // Enable FIFO, clear TX/RX queues and
    // set interrupt watermark at 14 bytes
    SERIAL_FIFO_CTRL.write(0xC7);
    // Mark data terminal ready, signal request to send
    // and enable auxilliary output #2 (used as interrupt line for CPU)
    SERIAL_MODEM_CTRL.write(0x0B);
    // Enable interrupts
    SERIAL_INT_EN.write(0x01);
}

pub fn register_serial_input_callback(f: impl Fn(u8) + Send + Sync + 'static) {
    SERIAL_INPUT_CALLBACKS.lock().push(Arc::new(f));
}

pub(crate) fn callback_init() {
    let mut irq = pic_allocate_irq(4).unwrap();
    irq.on_active(handle_serial_input);
    CONSOLE_IRQ_CALLBACK.call_once(|| Mutex::new(irq));
}

pub(crate) fn register_serial_input_irq_handler<F>(callback: F)
where
    F: Fn(&TrapFrame) + Sync + Send + 'static,
{
    CONSOLE_IRQ_CALLBACK
        .get()
        .unwrap()
        .lock()
        .on_active(callback);
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
    LineSts::from_bits_truncate(SERIAL_LINE_STS.read())
}

/// Sends a byte on the serial port.
pub fn send(data: u8) {
    match data {
        8 | 0x7F => {
            while !line_sts().contains(LineSts::OUTPUT_EMPTY) {}
            SERIAL_DATA.write(8);
            while !line_sts().contains(LineSts::OUTPUT_EMPTY) {}
            SERIAL_DATA.write(b' ');
            while !line_sts().contains(LineSts::OUTPUT_EMPTY) {}
            SERIAL_DATA.write(8);
        }
        _ => {
            while !line_sts().contains(LineSts::OUTPUT_EMPTY) {}
            SERIAL_DATA.write(data);
        }
    }
}

/// Receives a byte on the serial port. non-blocking
pub fn receive_char() -> Option<u8> {
    if line_sts().contains(LineSts::INPUT_FULL) {
        Some(SERIAL_DATA.read())
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
