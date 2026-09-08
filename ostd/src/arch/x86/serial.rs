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

    /// Detects whether a UART is present at the legacy COM1 serial port.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v7.2.2/source/drivers/tty/serial/8250/8250_port.c#L1094>
    fn probe(&mut self) -> bool {
        // A real UART echoes values written to its interrupt enable register, while
        // an unbacked port reads 0xFF or 0x00 on every access. We perform the
        // existence check by checking if the register works as expected.

        // Some UARTs (e.g., the TL 16C754B) only allow IER[7:4] to be modified when
        // an EFR bit is set, so only the low four bits are tested.
        const IER_ALL_INTR: u8 = 0x0F;

        let saved_ier = self.read(Ns16550aRegister::IntEnOrDivisorHi);

        let is_ok1 = {
            self.write(Ns16550aRegister::IntEnOrDivisorHi, 0x00);
            self.read(Ns16550aRegister::IntEnOrDivisorHi) & IER_ALL_INTR == 0
        };
        let is_ok2 = {
            self.write(Ns16550aRegister::IntEnOrDivisorHi, IER_ALL_INTR);
            self.read(Ns16550aRegister::IntEnOrDivisorHi) & IER_ALL_INTR == IER_ALL_INTR
        };

        self.write(Ns16550aRegister::IntEnOrDivisorHi, saved_ier);

        is_ok1 && is_ok2
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
///
/// # Safety
///
/// This function should be called only once.
pub(crate) unsafe fn init(early_cmdline: &EarlyCmdline) {
    if !early_cmdline.has_early_console {
        return;
    }

    // SAFETY:
    // 1. The legacy COM1 serial port at 0x3F8 can be disabled via the command line.
    //    (FIXME: This needs to be confirmed by checking the ACPI table or using more specific
    //    kernel parameters to obtain early information for building the early console.)
    // 2. `reserve_io_port_range` guarantees exclusive ownership of the I/O registers.
    let mut access = unsafe { SerialAccess::new(0x3F8) };
    if !access.probe() {
        // The `IoPort`s in the access are backed by statically reserved ranges,
        // but dropping the access would recycle these ranges through the port
        // allocator, which is not initialized yet at this point.
        core::mem::forget(access);
        return;
    }

    let mut serial = Ns16550aUart::new(access);
    serial.init();

    SERIAL_PORT.call_once(|| SpinLock::new(serial));
}
reserve_io_port_range!(0x3F8..0x400);
