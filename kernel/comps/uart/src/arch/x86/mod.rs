// SPDX-License-Identifier: MPL-2.0

use alloc::string::ToString;

use ostd::{
    arch::{
        irq::{IRQ_CHIP, MappedIrqLine},
        serial::SERIAL_PORT,
    },
    irq::IrqLine,
};
use spin::Once;

use crate::{
    CONSOLE_NAME,
    console::{Uart, UartConsole},
};

/// ISA interrupt number for UART serial.
// FIXME: The interrupt number should be retrieved from the ACPI table instead of being hard-coded.
const ISA_INTR_NUM: u8 = 4;

/// IRQ line for UART serial.
static IRQ_LINE: Once<MappedIrqLine> = Once::new();

pub(super) fn init() {
    let Some(uart) = SERIAL_PORT.get() else {
        return;
    };

    let Ok(mut irq_line) = IrqLine::alloc().and_then(|irq_line| {
        IRQ_CHIP
            .get()
            .unwrap()
            .map_isa_pin_to(irq_line, ISA_INTR_NUM)
    }) else {
        log::info!("[UART]: IRQ line is not available");
        return;
    };

    let uart_console = UartConsole::new(uart);

    aster_console::register_device(CONSOLE_NAME.to_string(), uart_console.clone());

    irq_line.on_active(move |_| uart_console.trigger_input_callbacks());
    IRQ_LINE.call_once(move || irq_line);
    uart.flush();

    log::info!("[UART]: Registered NS16550A as a console");
}
