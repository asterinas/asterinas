// SPDX-License-Identifier: MPL-2.0

use alloc::string::ToString;

use fdt::node::FdtNode;
use ostd::{
    arch::irq::{IRQ_CHIP, InterruptSourceInFdt, MappedIrqLine},
    console::uart_ns16650a::{Ns16550aAccess, Ns16550aRegister, Ns16550aUart},
    io::IoMem,
    irq::IrqLine,
    mm::VmIoOnce,
    sync::SpinLock,
};
use spin::Once;

use crate::{
    CONSOLE_NAME,
    console::{Uart, UartConsole},
};

/// Access to serial registers via `IoMem`.
struct SerialAccess {
    io_mem: IoMem,
}

impl Ns16550aAccess for SerialAccess {
    fn read(&self, reg: Ns16550aRegister) -> u8 {
        self.io_mem.read_once(reg as u16 as usize).unwrap()
    }

    fn write(&mut self, reg: Ns16550aRegister, val: u8) {
        self.io_mem.write_once(reg as u16 as usize, &val).unwrap();
    }
}

/// IRQ line for UART serial.
static IRQ_LINE: Once<MappedIrqLine> = Once::new();

pub(super) fn init(fdt_node: FdtNode) {
    let Some(reg) = fdt_node.reg().and_then(|mut regs| regs.next()) else {
        log::info!("[UART]: Failed to read 'reg' property from NS16550A node");
        return;
    };
    let Some(reg_size) = reg.size else {
        log::info!("[UART]: Incomplete 'reg' property found in NS16550A node");
        return;
    };

    let reg_addr = reg.starting_address as usize;
    let Ok(io_mem) = IoMem::acquire(reg_addr..reg_addr + reg_size) else {
        log::info!("[UART]: I/O memory is not available for NS16550A");
        return;
    };

    let Some(intr_parent) = fdt_node
        .property("interrupt-parent")
        .and_then(|prop| prop.as_usize())
    else {
        log::info!("[UART]: Failed to read 'interrupt-parent' property from NS16550A node");
        return;
    };
    let Some(intr) = fdt_node.interrupts().and_then(|mut intrs| intrs.next()) else {
        log::info!("[UART]: Failed to read 'interrupts' property from NS16550A node");
        return;
    };

    let Ok(mut irq_line) = IrqLine::alloc().and_then(|irq_line| {
        IRQ_CHIP.get().unwrap().map_fdt_pin_to(
            InterruptSourceInFdt {
                interrupt_parent: intr_parent as u32,
                interrupt: intr as u32,
            },
            irq_line,
        )
    }) else {
        log::info!("[UART]: IRQ line is not available for NS16550A");
        return;
    };

    let mut uart = Ns16550aUart::new(SerialAccess { io_mem });
    uart.init();

    let uart_console = UartConsole::new(SpinLock::new(uart));

    aster_console::register_device(CONSOLE_NAME.to_string(), uart_console.clone());

    let cloned_uart_console = uart_console.clone();
    irq_line.on_active(move |_| cloned_uart_console.trigger_input_callbacks());
    IRQ_LINE.call_once(move || irq_line);
    uart_console.uart().flush();

    log::info!("[UART]: Registered NS16550A as a console");
}
