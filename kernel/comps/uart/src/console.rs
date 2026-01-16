// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};
use core::fmt::Debug;

use aster_console::{AnyConsoleDevice, ConsoleCallback};
use inherit_methods_macro::inherit_methods;
use ostd::{
    console::uart_ns16650a::{Ns16550aAccess, Ns16550aUart},
    mm::VmReader,
    sync::{LocalIrqDisabled, SpinLock},
};

/// A UART console.
pub(super) struct UartConsole<U: Uart> {
    uart: U,
    callbacks: SpinLock<Vec<&'static ConsoleCallback>, LocalIrqDisabled>,
}

impl<U: Uart> Debug for UartConsole<U> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UartConsole").finish_non_exhaustive()
    }
}

impl<U: Uart> UartConsole<U> {
    /// Creates a new UART console.
    pub(super) fn new(uart: U) -> Arc<Self> {
        Arc::new(Self {
            uart,
            callbacks: SpinLock::new(Vec::new()),
        })
    }

    /// Returns a reference to the UART instance.
    #[cfg_attr(not(target_arch = "riscv64"), expect(dead_code))]
    pub(super) fn uart(&self) -> &U {
        &self.uart
    }

    // Triggers the registered input callbacks.
    pub(super) fn trigger_input_callbacks(&self) {
        let mut buf = [0; 16];

        loop {
            let num_rcv = self.uart.recv(&mut buf);
            if num_rcv == 0 {
                break;
            }

            let reader = VmReader::from(&buf[..num_rcv]);
            for callback in self.callbacks.lock().iter() {
                (callback)(reader.clone());
            }

            if num_rcv < buf.len() {
                break;
            }
        }
    }
}

impl<U: Uart + Send + Sync + 'static> AnyConsoleDevice for UartConsole<U> {
    fn send(&self, buf: &[u8]) {
        self.uart.send(buf);
    }

    fn register_callback(&self, callback: &'static ConsoleCallback) {
        self.callbacks.lock().push(callback);
    }
}

/// A trait that abstracts UART devices.
pub(super) trait Uart {
    /// Sends a sequence of bytes to UART.
    fn send(&self, buf: &[u8]);

    /// Receives a sequence of bytes from UART and returns the number of received bytes.
    #[must_use]
    fn recv(&self, buf: &mut [u8]) -> usize;

    /// Flushes the received buffer.
    ///
    /// This method should be called after setting up the IRQ handlers to ensure new received data
    /// will trigger IRQs.
    fn flush(&self);
}

impl<A: Ns16550aAccess> Uart for SpinLock<Ns16550aUart<A>, LocalIrqDisabled> {
    fn send(&self, buf: &[u8]) {
        let mut uart = self.lock();

        for byte in buf {
            // TODO: This is termios-specific behavior and should be part of the TTY implementation
            // instead of the serial console implementation. See the ONLCR flag for more details.
            if *byte == b'\n' {
                uart.send(b'\r');
            }
            uart.send(*byte);
        }
    }

    fn recv(&self, buf: &mut [u8]) -> usize {
        let mut uart = self.lock();

        for (i, byte) in buf.iter_mut().enumerate() {
            let Some(recv_byte) = uart.recv() else {
                return i;
            };
            *byte = recv_byte;
        }

        buf.len()
    }

    fn flush(&self) {
        let mut uart = self.lock();

        while uart.recv().is_some() {}
    }
}

#[inherit_methods(from = "(**self)")]
impl<A: Ns16550aAccess> Uart for &SpinLock<Ns16550aUart<A>, LocalIrqDisabled> {
    fn send(&self, buf: &[u8]);
    fn recv(&self, buf: &mut [u8]) -> usize;
    fn flush(&self);
}
