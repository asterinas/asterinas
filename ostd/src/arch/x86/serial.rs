// SPDX-License-Identifier: MPL-2.0

//! The console I/O.

use spin::Once;
use x86_64::instructions::port::ReadWriteAccess;

use crate::{
    console::uart_ns16650a::{Ns16550aAccess, Ns16550aRegister, Ns16550aUart},
    io::{IoPort, reserve_io_port_range},
    sync::{LocalIrqDisabled, SpinLock},
};

/// The primary serial port, which serves as an early console.
pub static SERIAL_PORT: Once<SpinLock<Ns16550aUart<SerialAccess>, LocalIrqDisabled>> =
    Once::initialized(SpinLock::new(Ns16550aUart::new(
        // SAFETY:
        // 1. It is assumed that the serial port exists and can be accessed via the I/O registers.
        //    (FIXME: This needs to be confirmed by checking the ACPI table or using kernel
        //    parameters to obtain early information for building the early console.)
        // 2. `reserve_io_port_range` guarantees exclusive ownership of the I/O registers.
        unsafe { SerialAccess::new(0x3F8) },
    )));
reserve_io_port_range!(0x3F8..0x400);

/// Access to serial registers via I/O ports in x86.
#[derive(Debug)]
pub struct SerialAccess {
    data: IoPort<u8, ReadWriteAccess>,
    int_en: IoPort<u8, ReadWriteAccess>,
    fifo_ctrl: IoPort<u8, ReadWriteAccess>,
    line_ctrl: IoPort<u8, ReadWriteAccess>,
    modem_ctrl: IoPort<u8, ReadWriteAccess>,
    line_stat: IoPort<u8, ReadWriteAccess>,
    modem_stat: IoPort<u8, ReadWriteAccess>,
}

impl SerialAccess {
    /// # Safety
    ///
    /// The caller must ensure that the base port is a valid serial base port and that it has
    /// exclusive ownership of the serial registers.
    const unsafe fn new(port: u16) -> Self {
        // SAFETY: The safety is upheld by the caller.
        unsafe {
            Self {
                data: IoPort::new(port),
                int_en: IoPort::new(port + 1),
                fifo_ctrl: IoPort::new(port + 2),
                line_ctrl: IoPort::new(port + 3),
                modem_ctrl: IoPort::new(port + 4),
                line_stat: IoPort::new(port + 5),
                modem_stat: IoPort::new(port + 6),
            }
        }
    }
}

impl Ns16550aAccess for SerialAccess {
    fn read(&self, reg: Ns16550aRegister) -> u8 {
        match reg {
            Ns16550aRegister::DataOrDivisorLo => self.data.read(),
            Ns16550aRegister::IntEnOrDivisorHi => self.int_en.read(),
            Ns16550aRegister::FifoCtrl => self.fifo_ctrl.read(),
            Ns16550aRegister::LineCtrl => self.line_ctrl.read(),
            Ns16550aRegister::ModemCtrl => self.modem_ctrl.read(),
            Ns16550aRegister::LineStat => self.line_stat.read(),
            Ns16550aRegister::ModemStat => self.modem_stat.read(),
        }
    }

    fn write(&mut self, reg: Ns16550aRegister, val: u8) {
        match reg {
            Ns16550aRegister::DataOrDivisorLo => self.data.write(val),
            Ns16550aRegister::IntEnOrDivisorHi => self.int_en.write(val),
            Ns16550aRegister::FifoCtrl => self.fifo_ctrl.write(val),
            Ns16550aRegister::LineCtrl => self.line_ctrl.write(val),
            Ns16550aRegister::ModemCtrl => self.modem_ctrl.write(val),
            Ns16550aRegister::LineStat => self.line_stat.write(val),
            Ns16550aRegister::ModemStat => self.modem_stat.write(val),
        }
    }
}

/// Initializes the serial port.
pub(crate) fn init() {
    SERIAL_PORT.get().unwrap().lock().init();
}
