//! A port-mapped UART. Copied from uart_16550.
use alloc::{sync::Arc, vec::Vec};
use log::debug;
use spin::{Mutex, Once};
use trapframe::TrapFrame;
use x86_64::instructions::port::{Port, PortWriteOnly};

use crate::{driver::pic_allocate_irq, trap::IrqAllocateHandle};
use core::fmt::{self, Write};

bitflags::bitflags! {
  struct LineSts: u8 {
    const INPUT_FULL = 1;
    const OUTPUT_EMPTY = 1 << 5;
  }
}

const SERIAL_DATA_PORT: u16 = 0x3F8;

static SERIAL_DATA: Mutex<Port<u8>> = Mutex::new(Port::new(SERIAL_DATA_PORT));
static SERIAL_INT_EN: Mutex<PortWriteOnly<u8>> =
    Mutex::new(PortWriteOnly::new(SERIAL_DATA_PORT + 1));
static SERIAL_FIFO_CTRL: Mutex<PortWriteOnly<u8>> =
    Mutex::new(PortWriteOnly::new(SERIAL_DATA_PORT + 2));
static SERIAL_LINE_CTRL: Mutex<PortWriteOnly<u8>> =
    Mutex::new(PortWriteOnly::new(SERIAL_DATA_PORT + 3));
static SERIAL_MODEM_CTRL: Mutex<PortWriteOnly<u8>> =
    Mutex::new(PortWriteOnly::new(SERIAL_DATA_PORT + 4));
static SERIAL_LINE_STS: Mutex<Port<u8>> = Mutex::new(Port::new(SERIAL_DATA_PORT + 5));

static CONSOLE_IRQ_CALLBACK: Once<Mutex<IrqAllocateHandle>> = Once::new();
static SERIAL_INPUT_CALLBACKS: Mutex<Vec<Arc<dyn Fn(u8) + Send + Sync + 'static>>> =
    Mutex::new(Vec::new());

/// Initializes the serial port.
pub(crate) fn init() {
    let mut serial_line_ctrl_lock = SERIAL_LINE_CTRL.lock();
    let mut serial_int_en_lock = SERIAL_INT_EN.lock();
    let mut serial_data_lock = SERIAL_DATA.lock();
    let mut serial_fifo_ctrl_lock = SERIAL_FIFO_CTRL.lock();
    let mut serial_modem_ctrl_lock = SERIAL_MODEM_CTRL.lock();
    unsafe {
        // Disable interrupts
        serial_int_en_lock.write(0x00);
        // Enable DLAB
        serial_line_ctrl_lock.write(0x80);
        // Set maximum speed to 38400 bps by configuring DLL and DLM
        serial_data_lock.write(0x03);
        serial_int_en_lock.write(0x00);
        // Disable DLAB and set data word length to 8 bits
        serial_line_ctrl_lock.write(0x03);
        // Enable FIFO, clear TX/RX queues and
        // set interrupt watermark at 14 bytes
        serial_fifo_ctrl_lock.write(0xC7);
        // Mark data terminal ready, signal request to send
        // and enable auxilliary output #2 (used as interrupt line for CPU)
        serial_modem_ctrl_lock.write(0x0B);
        // Enable interrupts
        serial_int_en_lock.write(0x01);
    }
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
    LineSts::from_bits_truncate(unsafe { SERIAL_LINE_STS.lock().read() })
}

/// Sends a byte on the serial port.
pub fn send(data: u8) {
    let mut lock = SERIAL_DATA.lock();
    unsafe {
        match data {
            8 | 0x7F => {
                while !line_sts().contains(LineSts::OUTPUT_EMPTY) {}
                lock.write(8);
                while !line_sts().contains(LineSts::OUTPUT_EMPTY) {}
                lock.write(b' ');
                while !line_sts().contains(LineSts::OUTPUT_EMPTY) {}
                lock.write(8);
            }
            _ => {
                while !line_sts().contains(LineSts::OUTPUT_EMPTY) {}
                lock.write(data);
            }
        }
    }
}

/// Receives a byte on the serial port. non-blocking
pub fn receive_char() -> Option<u8> {
    if line_sts().contains(LineSts::INPUT_FULL) {
        unsafe { Some(SERIAL_DATA.lock().read()) }
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
