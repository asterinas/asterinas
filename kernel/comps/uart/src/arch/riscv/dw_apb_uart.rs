// SPDX-License-Identifier: MPL-2.0
//! DesignWare APB UART support for RISC-V.
//!
//! This module adapts the DesignWare APB register layout to
//! [`Ns16550aUart`], configures its baud-rate divisor, and connects
//! receive interrupts to the RISC-V interrupt controller.
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

const DEFAULT_BAUD_RATE: usize = 115_200;
/// Level-high trigger flag in Linux-compatible device trees.
const IRQ_TYPE_LEVEL_HIGH: u32 = 4;
/// The width of one MMIO access to a UART register.
#[derive(Clone, Copy)]
enum RegIoWidth {
    U8,
    U32,
}
impl RegIoWidth {
    const fn bytes(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U32 => 4,
        }
    }
}
/// Access to serial registers via `IoMem`.
struct DwApbUartAccess {
    io_mem: IoMem,
    reg_shift: u32,
    reg_io_width: RegIoWidth,
}

const DW_UART_USR_INDEX: usize = 0x1f;
const DW_UART_USR_BUSY: u32 = 1 << 0;
const DW_UART_BUSY_POLL_LIMIT: usize = 1_000_000;

impl DwApbUartAccess {
    fn offset(&self, reg: Ns16550aRegister) -> usize {
        (reg as u16 as usize) << self.reg_shift
    }
    fn read_raw(&self, register_index: usize) -> u32 {
        let offset = register_index << self.reg_shift;

        match self.reg_io_width {
            RegIoWidth::U8 => {
                let value: u8 = self.io_mem.read_once(offset).unwrap();
                u32::from(value)
            }
            RegIoWidth::U32 => self.io_mem.read_once(offset).unwrap(),
        }
    }

    fn wait_until_idle(&self) -> bool {
        for _ in 0..DW_UART_BUSY_POLL_LIMIT {
            if self.read_raw(DW_UART_USR_INDEX) & DW_UART_USR_BUSY == 0 {
                return true;
            }

            core::hint::spin_loop();
        }

        false
    }

    fn clear_busy_interrupt(&self) {
        // Reading USR clears a pending DW UART Busy Detect interrupt.
        let _ = self.read_raw(DW_UART_USR_INDEX);
    }
}

impl Ns16550aAccess for DwApbUartAccess {
    fn read(&self, reg: Ns16550aRegister) -> u8 {
        let offset = self.offset(reg);
        match self.reg_io_width {
            RegIoWidth::U8 => self.io_mem.read_once(offset).unwrap(),
            RegIoWidth::U32 => {
                let value: u32 = self.io_mem.read_once(offset).unwrap();
                value as u8
            }
        }
    }

    fn write(&mut self, reg: Ns16550aRegister, val: u8) {
        let offset = self.offset(reg);
        match self.reg_io_width {
            RegIoWidth::U8 => self.io_mem.write_once(offset, &val).unwrap(),
            RegIoWidth::U32 => {
                let value: u32 = u32::from(val);
                self.io_mem.write_once(offset, &value).unwrap();
            }
        }
    }
}

fn calculate_divisor(clock_frequency: usize, baud_rate: usize) -> Option<u16> {
    let denominator = baud_rate.checked_mul(16)?;
    let rounded_clock = clock_frequency.checked_add(denominator / 2)?;
    let divisor = rounded_clock.checked_div(denominator)?;
    if divisor == 0 {
        return None;
    }
    u16::try_from(divisor).ok()
}

fn parse_interrupt(fdt_node: FdtNode) -> Option<(u32, Option<u32>)> {
    let interrupt_cells = fdt_node.interrupt_parent()?.interrupt_cells()?;
    let property = fdt_node.property("interrupts")?;
    let (cells, remainder) = property.value.as_chunks::<4>();

    if !remainder.is_empty() {
        return None;
    }

    match (interrupt_cells, cells) {
        (1, [interrupt]) => Some((u32::from_be_bytes(*interrupt), None)),
        (2, [interrupt, trigger_type]) => Some((
            u32::from_be_bytes(*interrupt),
            Some(u32::from_be_bytes(*trigger_type)),
        )),
        _ => None,
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
        ostd::info!("Incomplete 'reg' property found in DW APB UART node");
        return;
    };

    let reg_addr = reg.starting_address as usize;
    let Some(reg_end) = reg_addr.checked_add(reg_size) else {
        ostd::info!("DW APB UART MMIO range overflows the address space");
        return;
    };
    let reg_shift = fdt_node
        .property("reg-shift")
        .and_then(|property| property.as_usize())
        .unwrap_or(0);
    let Ok(reg_shift) = u32::try_from(reg_shift) else {
        ostd::info!("Invalid DW APB UART 'reg_shift' property");
        return;
    };
    let reg_io_width = fdt_node
        .property("reg-io-width")
        .and_then(|property| property.as_usize())
        .unwrap_or(1);
    let reg_io_width = match reg_io_width {
        1 => RegIoWidth::U8,
        4 => RegIoWidth::U32,
        width => {
            ostd::info!("Unsupported DW APB UART reg-io-width:{}", width);
            return;
        }
    };
    let Some(register_stride) = 1usize.checked_shl(reg_shift) else {
        ostd::info!("DW APB UART register shift is too large");
        return;
    };

    let access_width = reg_io_width.bytes();

    if register_stride < access_width || !reg_addr.is_multiple_of(access_width) {
        ostd::info!("DW APB UART register layout is not properly aligned");
        return;
    }

    let Some(last_register_offset) = DW_UART_USR_INDEX.checked_mul(register_stride) else {
        ostd::info!("DW APB UART register offset overflows");
        return;
    };

    let Some(required_reg_size) = last_register_offset.checked_add(reg_io_width.bytes()) else {
        ostd::info!("DW APB UART register range overflows");
        return;
    };
    if required_reg_size > reg_size {
        ostd::info!(
            "DW APB UART register range is too small:required={:#},actual={:#}",
            required_reg_size,
            reg_size
        );
        return;
    }
    let clock_frequency = fdt_node
        .property("clock-frequency")
        .and_then(|property| property.as_usize());
    let Some(clock_frequency) = clock_frequency else {
        ostd::info!("Missing 'clock-frequency' property from DW APB UART node");
        return;
    };
    let baud_rate = fdt_node
        .property("current-speed")
        .and_then(|property| property.as_usize())
        .unwrap_or(DEFAULT_BAUD_RATE);
    let Some(divisor) = calculate_divisor(clock_frequency, baud_rate) else {
        ostd::info!(
            "cannot calculate DW APB UART divisor:clock={},baud={}",
            clock_frequency,
            baud_rate
        );
        return;
    };

    let Ok(io_mem) = IoMem::acquire(reg_addr..reg_end) else {
        ostd::info!("I/O memory is not available for DW APB UART");
        return;
    };

    let Some(intr_parent) = fdt_node
        .property("interrupt-parent")
        .and_then(|prop| prop.as_usize())
    else {
        ostd::info!("Failed to read 'interrupt-parent' property from DW APB UART node");
        return;
    };

    let Some((intr, trigger_type)) = parse_interrupt(fdt_node) else {
        ostd::info!("Invalid 'interrupts' property in DW APB UART node");
        return;
    };

    if let Some(trigger_type) = trigger_type
        && trigger_type != IRQ_TYPE_LEVEL_HIGH
    {
        ostd::info!("Unsupported DW APB UART IRQ trigger type: {}", trigger_type);
        return;
    }

    let access = DwApbUartAccess {
        io_mem,
        reg_shift,
        reg_io_width,
    };
    if !access.wait_until_idle() {
        ostd::warn!("DW APB UART did not become idle");
        return;
    }

    access.clear_busy_interrupt();
    let mut uart = Ns16550aUart::new(access);

    uart.init_with_divisor(divisor);

    let uart_console = UartConsole::new(SpinLock::new(uart));
    uart_console.uart().flush();

    let mut irq_line = match IrqLine::alloc() {
        Ok(line) => line,
        Err(_) => {
            ostd::warn!("failed to allocate IRQ line for DW APB UART");
            return;
        }
    };
    // Install the callback before mapping the PLIC source. Mapping enables
    // delivery, so reversing the order would leave a window without a handler.
    let cloned_uart_console = uart_console.clone();
    irq_line.on_active(move |_| cloned_uart_console.trigger_input_callbacks());

    let mapped_irq_line = match IRQ_CHIP.get().unwrap().map_fdt_pin_to(
        InterruptSourceInFdt {
            interrupt_parent: intr_parent as u32,
            arguments: [intr],
        },
        irq_line,
    ) {
        Ok(line) => line,
        Err(_) => {
            ostd::warn!("failed to map DW APB UART IRQ {}", intr);
            return;
        }
    };
    // Retain the mapped line for the lifetime of the console. Dropping it
    // would automatically unmap and disable the PLIC source.
    IRQ_LINE.call_once(move || mapped_irq_line);
    aster_console::register_device(
        aster_console::UART_CONSOLE_NAME.to_string(),
        uart_console.clone(),
    );
    uart_console.uart().lock().enable_receive_interrupt();
    ostd::info!(
        "Registered DW APB UART with PLIC IRQ {} mapped, UART RX IRQ enabled",
        intr
    );
}
