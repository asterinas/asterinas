// SPDX-License-Identifier: MPL-2.0

use ostd::{
    arch::serial::{Pl011, SERIAL_PORT},
    sync::{LocalIrqDisabled, SpinLock},
};

use crate::{
    CONSOLE_NAME,
    alloc::string::ToString,
    console::{Uart, UartConsole},
};

impl Uart for &SpinLock<Pl011, LocalIrqDisabled> {
    fn send(&self, buf: &[u8]) {
        let uart = self.lock();
        for byte in buf {
            // TODO: This is termios-specific behavior and should live in the TTY
            // layer. See the ONLCR flag.
            if *byte == b'\n' {
                uart.send(b'\r');
            }
            uart.send(*byte);
        }
    }

    fn recv(&self, buf: &mut [u8]) -> usize {
        let uart = self.lock();
        for (i, byte) in buf.iter_mut().enumerate() {
            let Some(recv_byte) = uart.recv() else {
                return i;
            };
            *byte = recv_byte;
        }
        buf.len()
    }

    fn flush(&self) {
        let uart = self.lock();
        while uart.recv().is_some() {}
    }
}

pub(super) fn init() {
    let Some(uart) = SERIAL_PORT.get() else {
        return;
    };

    let uart_console = UartConsole::new(uart);

    aster_console::register_device(CONSOLE_NAME.to_string(), uart_console.clone());

    // TODO: Set up the IRQ line and handle received data.
    let _ = || uart_console.trigger_input_callbacks();
    let _ = || uart.flush();

    ostd::info!("Registered PL011 as a console");
}
