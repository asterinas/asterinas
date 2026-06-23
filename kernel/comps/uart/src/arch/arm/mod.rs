// SPDX-License-Identifier: MPL-2.0

use ostd::arch::serial::{Pl011Uart, SERIAL_PORT};

use crate::{
    alloc::string::ToString,
    console::{Uart, UartConsole, UartMut},
};

pub(super) fn init() {
    let Some(uart) = SERIAL_PORT.get() else {
        return;
    };

    let uart_console = UartConsole::new(uart);

    aster_console::register_device(
        aster_console::UART_CONSOLE_NAME.to_string(),
        uart_console.clone(),
    );

    // TODO: Set up the IRQ line and handle the received data.
    // Suppress the dead code warnings of the related methods.
    let _ = || uart_console.trigger_input_callbacks();
    let _ = || uart.flush();

    ostd::info!("Registered PL011 as a console");
}

impl UartMut for Pl011Uart {
    fn send_byte(&mut self, byte: u8) {
        self.send(byte);
    }

    fn recv_byte(&mut self) -> Option<u8> {
        // TODO: Set up the IRQ line and handle the received data.
        None
    }
}
