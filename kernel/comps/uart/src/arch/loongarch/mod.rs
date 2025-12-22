// SPDX-License-Identifier: MPL-2.0

use ostd::arch::serial::SERIAL_PORT;

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

    log::info!("[UART]: Registered NS16550A as a console");
}
