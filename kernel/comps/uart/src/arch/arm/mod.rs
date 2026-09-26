// SPDX-License-Identifier: MPL-2.0

use fdt_util::AcquireIrqLines;
use ostd::arch::{
    irq::MappedIrqLine,
    serial::{Pl011Uart, SERIAL_PORT},
};
use spin::Once;

use crate::{
    alloc::string::ToString,
    console::{Uart, UartConsole, UartMut},
};

/// IRQ line for UART serial.
static IRQ_LINE: Once<MappedIrqLine> = Once::new();

pub(super) fn init() {
    let Some(uart) = SERIAL_PORT.get() else {
        return;
    };

    let node = uart.lock().fdt_node();
    let Some([mut irq_line]) = node.acquire_irq_lines() else {
        return;
    };

    let uart_console = UartConsole::new(uart);

    aster_console::register_device(
        aster_console::UART_CONSOLE_NAME.to_string(),
        uart_console.clone(),
    );

    irq_line.on_active(move |_| uart_console.trigger_input_callbacks());
    IRQ_LINE.call_once(move || irq_line);
    uart.lock().enable_recv_interrupt();
    uart.flush();

    ostd::info!("Registered PL011 as a console");
}

impl UartMut for Pl011Uart {
    fn send_byte(&mut self, byte: u8) {
        self.send(byte);
    }

    fn recv_byte(&mut self) -> Option<u8> {
        self.recv()
    }
}
