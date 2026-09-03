// SPDX-License-Identifier: MPL-2.0

//! The console I/O.

use spin::Once;
use x86_64::instructions::port::ReadWriteAccess;

use crate::{
    boot::EarlyCmdline,
    console::uart_ns16650a::{Ns16550aAccess, Ns16550aRegister, Ns16550aUart},
    io::{IoPort, reserve_io_port_range},
    sync::{LocalIrqDisabled, SpinLock},
};

/// The primary serial port, which serves as an early console.
pub static SERIAL_PORT: Once<SpinLock<Ns16550aUart<SerialAccess>, LocalIrqDisabled>> = Once::new();

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

/// Detects whether a UART is present at the legacy COM1 serial port.
/// Reference: <https://elixir.bootlin.com/linux/v7.2.2/source/drivers/tty/serial/8250/8250_port.c#L1094>
pub(crate) fn probe_serial_port() -> bool {
    // SAFETY:
    // 1. 0x3F8 is the legacy COM1 serial port we use in SERIAL_PORT.
    // 2. `reserve_io_port_range` guarantees exclusive ownership of the I/O registers.
    let mut access = unsafe { SerialAccess::new(0x3F8) };

    // A real UART echoes values written to its interrupt enable register, while
    // an unbacked port reads 0xFF or 0x00 on every access. We perform the
    // existence check similar to the Linux implementation: write 0x00 and then
    // 0x0f and check that both are read back. Some UARTs (e.g. the TL 16C754B)
    // only allow IER[7:4] to be modified when an EFR bit is set, so only the
    // low four bits are compared.
    const IER_ALL_INTR: u8 = 0x0f;
    let saved_ier = access.read(Ns16550aRegister::IntEnOrDivisorHi);
    access.write(Ns16550aRegister::IntEnOrDivisorHi, 0x00);
    let val1 = access.read(Ns16550aRegister::IntEnOrDivisorHi) & IER_ALL_INTR;
    access.write(Ns16550aRegister::IntEnOrDivisorHi, IER_ALL_INTR);
    let val2 = access.read(Ns16550aRegister::IntEnOrDivisorHi) & IER_ALL_INTR;
    access.write(Ns16550aRegister::IntEnOrDivisorHi, saved_ier);
    val1 == 0x00 && val2 == IER_ALL_INTR
}

/// Initializes the serial port.
pub(crate) fn init(early_cmdline: &EarlyCmdline) {
    if !early_cmdline.has_early_console {
        return;
    }

    if !probe_serial_port() {
        return;
    }

    SERIAL_PORT.call_once(|| {
        // SAFETY:
        // 1. The legacy COM1 serial port at 0x3F8 can be disabled via the command line.
        //    (FIXME: This needs to be confirmed by checking the ACPI table or using more specific
        //    kernel parameters to obtain early information for building the early console.)
        // 2. `reserve_io_port_range` guarantees exclusive ownership of the I/O registers.
        let access = unsafe { SerialAccess::new(0x3F8) };
        let mut serial = Ns16550aUart::new(access);
        serial.init();
        SpinLock::new(serial)
    });
}
reserve_io_port_range!(0x3F8..0x400);
