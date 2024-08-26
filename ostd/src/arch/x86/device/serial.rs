// SPDX-License-Identifier: MPL-2.0

//! A port-mapped UART. Copied from uart_16550.

#![allow(dead_code)]

use crate::arch::x86::device::io_port::{IoPort, ReadWriteAccess, WriteOnlyAccess};

/// A serial port.
///
/// Serial ports are a legacy communications port common on IBM-PC compatible computers.
/// Ref: <https://wiki.osdev.org/Serial_Ports>
pub struct SerialPort {
    /// Data Register
    data: IoPort<u8, ReadWriteAccess>,
    /// Interrupt Enable Register
    int_en: IoPort<u8, WriteOnlyAccess>,
    /// First In First Out Control Register
    fifo_ctrl: IoPort<u8, WriteOnlyAccess>,
    /// Line control Register
    line_ctrl: IoPort<u8, WriteOnlyAccess>,
    /// Modem Control Register
    modem_ctrl: IoPort<u8, WriteOnlyAccess>,
    /// Line status Register
    line_status: IoPort<u8, ReadWriteAccess>,
    /// Modem Status Register
    modem_status: IoPort<u8, ReadWriteAccess>,
}

impl SerialPort {
    /// Creates a serial port.
    ///
    /// # Safety
    ///
    /// User must ensure the `port` is valid serial port.
    pub const unsafe fn new(port: u16) -> Self {
        let data = IoPort::new(port);
        let int_en = IoPort::new(port + 1);
        let fifo_ctrl = IoPort::new(port + 2);
        let line_ctrl = IoPort::new(port + 3);
        let modem_ctrl = IoPort::new(port + 4);
        let line_status = IoPort::new(port + 5);
        let modem_status = IoPort::new(port + 6);
        Self {
            data,
            int_en,
            fifo_ctrl,
            line_ctrl,
            modem_ctrl,
            line_status,
            modem_status,
        }
    }

    /// Initializes the serial port.
    pub fn init(&self) {
        // Disable interrupts
        self.int_en.write(0x00);
        // Enable DLAB
        self.line_ctrl.write(0x80);
        // Set maximum speed to 38400 bps by configuring DLL and DLM
        self.data.write(0x03);
        self.int_en.write(0x00);
        // Disable DLAB and set data word length to 8 bits
        self.line_ctrl.write(0x03);
        // Enable FIFO, clear TX/RX queues and
        // set interrupt watermark at 14 bytes
        self.fifo_ctrl.write(0xC7);
        // Mark data terminal ready, signal request to send
        // and enable auxiliary output #2 (used as interrupt line for CPU)
        self.modem_ctrl.write(0x0B);
        // Enable interrupts
        self.int_en.write(0x01);
    }

    /// Sends data to the data port
    #[inline]
    pub fn send(&self, data: u8) {
        self.data.write(data);
    }

    /// Receives data from the data port
    #[inline]
    pub fn recv(&self) -> u8 {
        self.data.read()
    }

    /// Gets line status
    #[inline]
    pub fn line_status(&self) -> u8 {
        self.line_status.read()
    }
}
