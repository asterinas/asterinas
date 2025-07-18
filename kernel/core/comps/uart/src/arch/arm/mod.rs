// SPDX-License-Identifier: MPL-2.0

use ostd::{
    arch::serial::{Pl011Uart, SERIAL_PORT},
    sync::{LocalIrqDisabled, SpinLock},
};

use crate::{
    CONSOLE_NAME,
    alloc::string::ToString,
    console::{Uart, UartConsole},
};

pub(super) fn init() {
    let Some(uart) = SERIAL_PORT.get() else {
        return;
    };

    let uart_console = UartConsole::new(uart);

    aster_console::register_device(CONSOLE_NAME.to_string(), uart_console.clone());

    // TODO: Set up the IRQ line and handle the received data.
    // Suppress the dead code warnings of the related methods.
    let _ = || uart_console.trigger_input_callbacks();
    let _ = || uart.flush();

    ostd::info!("Registered PL011 as a console");
}

impl Uart for &'static SpinLock<Pl011Uart, LocalIrqDisabled> {
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

    fn recv(&self, _buf: &mut [u8]) -> usize {
        // TODO: Set up the IRQ line and handle the received data.
        0
    }

    fn flush(&self) {
        // TODO: Set up the IRQ line and flush the received data.
    }
}
