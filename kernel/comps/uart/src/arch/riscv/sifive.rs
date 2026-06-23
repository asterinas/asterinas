// SPDX-License-Identifier: MPL-2.0

//! SiFive UART console support.

use alloc::string::ToString;

use aster_util::{field_ptr, safe_ptr::SafePtr};
use fdt::node::FdtNode;
use ostd::{
    arch::irq::{self, InterruptSourceInFdt, MappedIrqLine},
    io::IoMem,
    irq::IrqLine,
    sync::{LocalIrqDisabled, SpinLock},
};
use spin::Once;

use crate::{
    CONSOLE_NAME,
    console::{Uart, UartConsole, UartMut},
};

pub(super) const FDT_COMPATIBLES: [&str; 2] = ["sifive,uart0", "sifive,fu540-c000-uart"];

/// IRQ line for UART serial.
static IRQ_LINE: Once<MappedIrqLine> = Once::new();

pub(super) fn init(fdt_node: FdtNode) {
    let Some(reg) = fdt_node.reg().and_then(|mut regs| regs.next()) else {
        ostd::info!("Failed to read 'reg' property from SiFive UART node");
        return;
    };
    let Some(reg_size) = reg.size else {
        ostd::info!("Incomplete 'reg' property found in SiFive UART node");
        return;
    };

    let reg_addr = reg.starting_address as usize;
    let Some(reg_end) = reg_addr.checked_add(reg_size) else {
        ostd::info!("Invalid I/O memory range found in SiFive UART node");
        return;
    };
    let Ok(io_mem) = IoMem::acquire(reg_addr..reg_end) else {
        ostd::info!("I/O memory is not available for SiFive UART");
        return;
    };

    let Some(intr_parent) = fdt_node
        .property("interrupt-parent")
        .and_then(|prop| prop.as_usize())
    else {
        ostd::info!("Failed to read 'interrupt-parent' property from SiFive UART node");
        return;
    };
    let Some(intr) = fdt_node.interrupts().and_then(|mut intrs| intrs.next()) else {
        ostd::info!("Failed to read 'interrupts' property from SiFive UART node");
        return;
    };

    let Ok(mut irq_line) = IrqLine::alloc().and_then(|irq_line| {
        irq::IRQ_CHIP.get().unwrap().map_fdt_pin_to(
            InterruptSourceInFdt {
                interrupt_parent: intr_parent as u32,
                interrupt: intr as u32,
            },
            irq_line,
        )
    }) else {
        ostd::info!("IRQ line is not available for SiFive UART");
        return;
    };

    let clock_hz = fdt_node
        .property("clock-frequency")
        .and_then(|prop| prop.as_usize())
        .and_then(|freq| u32::try_from(freq).ok())
        // FIXME: Real FU540 device trees may provide the UART clock through
        // a PRCI clock provider instead of a direct `clock-frequency` property.
        .unwrap_or(SifiveUart::DEFAULT_CLOCK_HZ);

    let mut uart: SpinLock<SifiveUart, LocalIrqDisabled> = SpinLock::new(SifiveUart::new(io_mem));
    uart.get_mut().init(clock_hz);

    let uart_console = UartConsole::new(uart);

    aster_console::register_device(CONSOLE_NAME.to_string(), uart_console.clone());

    let cloned_uart_console = uart_console.clone();
    irq_line.on_active(move |_| cloned_uart_console.trigger_input_callbacks());
    IRQ_LINE.call_once(move || irq_line);
    uart_console.uart().flush();

    ostd::info!("Registered SiFive UART as a console");
}

struct SifiveUart {
    registers: SafePtr<Registers, IoMem>,
}

impl SifiveUart {
    const DEFAULT_CLOCK_HZ: u32 = 500_000_000;
    const TARGET_BAUD: u32 = 115_200;

    const TXDATA_FULL: u32 = 1 << 31;
    const RXDATA_EMPTY: u32 = 1 << 31;
    const TXCTRL_TXEN: u32 = 1;
    const RXCTRL_RXEN: u32 = 1;
    const IE_RXWM: u32 = 1 << 1;

    fn new(io_mem: IoMem) -> Self {
        Self {
            registers: SafePtr::new(io_mem, 0),
        }
    }

    fn init(&mut self, clock_hz: u32) {
        let divisor = clock_hz
            .checked_div(Self::TARGET_BAUD)
            .and_then(|quotient| quotient.checked_sub(1))
            .unwrap_or(0);

        field_ptr!(&self.registers, Registers, div)
            .write_once(&divisor)
            .unwrap();
        field_ptr!(&self.registers, Registers, txctrl)
            .write_once(&Self::TXCTRL_TXEN)
            .unwrap();
        field_ptr!(&self.registers, Registers, rxctrl)
            .write_once(&Self::RXCTRL_RXEN)
            .unwrap();
        field_ptr!(&self.registers, Registers, ie)
            .write_once(&Self::IE_RXWM)
            .unwrap();
    }
}

impl UartMut for SifiveUart {
    fn send_byte(&mut self, byte: u8) {
        let txdata = field_ptr!(&self.registers, Registers, txdata);
        while txdata.read_once().unwrap() & Self::TXDATA_FULL != 0 {
            core::hint::spin_loop();
        }

        txdata.write_once(&u32::from(byte)).unwrap();
    }

    fn recv_byte(&mut self) -> Option<u8> {
        let rxdata = field_ptr!(&self.registers, Registers, rxdata)
            .read_once()
            .unwrap();
        if rxdata & Self::RXDATA_EMPTY != 0 {
            return None;
        }

        Some(rxdata as u8)
    }
}

// Register layout and bit definitions are from the SiFive FU540-C000 Manual,
// Chapter 13 "Universal Asynchronous Receiver/Transmitter (UART)".
// Reference: <https://pdos.csail.mit.edu/6.828/2025/readings/FU540-C000-v1.0.pdf>
#[repr(C)]
struct Registers {
    txdata: u32,
    rxdata: u32,
    txctrl: u32,
    rxctrl: u32,
    ie: u32,
    _ip: u32,
    div: u32,
}
