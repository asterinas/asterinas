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

use crate::console::{Uart, UartConsole};

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
        ostd::info!("Failed to read 'reg' property from NS16550A node");
        return;
    };
    let Some(reg_size) = reg.size else {
        ostd::info!("Incomplete 'reg' property found in NS16550A node");
        return;
    };

    let reg_addr = reg.starting_address as usize;
    let Ok(io_mem) = IoMem::acquire(reg_addr..reg_addr + reg_size) else {
        ostd::info!("I/O memory is not available for NS16550A");
        return;
    };

    let Some(intr_parent) = fdt_node
        .property("interrupt-parent")
        .and_then(|prop| prop.as_usize())
    else {
        ostd::info!("Failed to read 'interrupt-parent' property from NS16550A node");
        return;
    };
    let intr_args = if let Some(prop) = fdt_node.property("interrupts")
        && let Ok(args) = prop
            .value
            .as_chunks::<{ size_of::<u32>() }>()
            .0
            .iter()
            .map(|chunk| u32::from_be_bytes(*chunk))
            .next_chunk()
    {
        args
    } else {
        ostd::info!("Failed to read 'interrupts' property from NS16550A node");
        return;
    };

    let Ok(mut irq_line) = IrqLine::alloc().and_then(|irq_line| {
        IRQ_CHIP.get().unwrap().map_fdt_pin_to(
            InterruptSourceInFdt {
                interrupt_parent: intr_parent as u32,
                arguments: intr_args,
            },
            irq_line,
        )
    }) else {
        ostd::info!("IRQ line is not available for NS16550A");
        return;
    };

    let mut uart = Ns16550aUart::new(SerialAccess { io_mem });
    uart.init();

    let uart_console = UartConsole::new(SpinLock::new(uart));

    aster_console::register_device(
        aster_console::UART_CONSOLE_NAME.to_string(),
        uart_console.clone(),
    );

    let cloned_uart_console = uart_console.clone();
    irq_line.on_active(move |_| cloned_uart_console.trigger_input_callbacks());
    IRQ_LINE.call_once(move || irq_line);
    uart_console.uart().flush();

    ostd::info!("Registered NS16550A as a console");
}
