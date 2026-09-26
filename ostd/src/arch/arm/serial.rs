// SPDX-License-Identifier: MPL-2.0

//! The console I/O.

use core::fmt;

use bitflags::bitflags;
use fdt::node::FdtNode;
use spin::Once;

use super::boot::DEVICE_TREE;
use crate::{
    boot::EarlyCmdline,
    mm::paddr_to_vaddr,
    sync::{LocalIrqDisabled, SpinLock},
};

/// The primary serial port, which serves as an early console.
pub static SERIAL_PORT: Once<SpinLock<Pl011Uart, LocalIrqDisabled>> = Once::new();

/// A PL011 serial port.
pub struct Pl011Uart {
    base: *mut u32,
    node: FdtNode<'static, 'static>,
}

unsafe impl Send for Pl011Uart {}
unsafe impl Sync for Pl011Uart {}

bitflags! {
    // Reference: <https://developer.arm.com/documentation/ddi0183/g/programmers-model/register-descriptions/flag-register--uartfr>
    struct Status: u32 {
        /// Transmit FIFO full.
        const TXFF = 1 << 5;
        /// Receive FIFO empty.
        const RXFE = 1 << 4;
    }
}

impl Pl011Uart {
    // Reference: <https://developer.arm.com/documentation/ddi0183/g/programmers-model/summary-of-registers>
    const OFFSET_UARTDR: usize = 0x000; // Data Register.
    const OFFSET_UARTFR: usize = 0x018; // Flag Register.
    const OFFSET_UARTIMSC: usize = 0x038; // Interrupt Mask Set/Clear Register.

    /// # Safety
    ///
    /// The caller must ensure that the base address is a valid serial base address and that it has
    /// exclusive ownership of the serial registers.
    pub(self) unsafe fn new(base: *mut u32, node: FdtNode<'static, 'static>) -> Self {
        Self { base, node }
    }

    /// Returns the FDT node of the serial port.
    pub fn fdt_node(&self) -> FdtNode<'static, 'static> {
        self.node
    }

    /// Enables receive interrupts.
    pub fn enable_recv_interrupt(&mut self) {
        // Reference: <https://developer.arm.com/documentation/ddi0183/g/programmers-model/register-descriptions/interrupt-mask-set-clear-register--uartimsc>
        const RXIM: u32 = 1 << 4; // Receive interrupt mask.

        // SAFETY: `base + OFFSET_UARTIMSC` is a valid register of the serial port.
        unsafe {
            let uartimsc = self.base.byte_add(Self::OFFSET_UARTIMSC);
            let val = uartimsc.read_volatile();
            uartimsc.write_volatile(val | RXIM);
        }
    }

    /// Sends a byte to the serial port.
    pub fn send(&mut self, byte: u8) {
        while self.read_status().contains(Status::TXFF) {
            core::hint::spin_loop();
        }
        self.write_data(byte);
    }

    /// Receives a byte from the serial port.
    pub fn recv(&mut self) -> Option<u8> {
        if self.read_status().contains(Status::RXFE) {
            None
        } else {
            Some(self.read_data())
        }
    }

    fn write_data(&mut self, data: u8) {
        // SAFETY: `base + OFFSET_UARTDR` is a valid register of the serial port.
        unsafe {
            self.base
                .byte_add(Self::OFFSET_UARTDR)
                .write_volatile(data as u32);
        }
    }

    fn read_data(&mut self) -> u8 {
        // SAFETY: `base + OFFSET_UARTDR` is a valid register of the serial port.
        unsafe { self.base.byte_add(Self::OFFSET_UARTDR).read_volatile() as u8 }
    }

    fn read_status(&self) -> Status {
        // SAFETY: `base + OFFSET_UARTFR` is a valid register of the serial port.
        let raw = unsafe { self.base.byte_add(Self::OFFSET_UARTFR).read_volatile() };
        Status::from_bits_truncate(raw)
    }
}

impl fmt::Write for Pl011Uart {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.as_bytes() {
            self.send(*c);
        }
        Ok(())
    }
}

/// Initializes the serial port.
pub(crate) fn init(early_cmdline: &EarlyCmdline) {
    if !early_cmdline.has_early_console {
        return;
    }

    let Some((base_address, fdt_node)) = lookup_pl011_base_address() else {
        return;
    };

    // SAFETY:
    // 1. The base address is valid and correct because it is acquired from the device tree.
    // 2. FIXME: We should ensure the address region is mapped in the boot page table and it
    //    has a correct memory attribute (i.e., device memory).
    // 3. FIXME: We should reserve the address region in `io_mem_allocator` to ensure the
    //    exclusive ownership.
    let pl011 = unsafe { Pl011Uart::new(paddr_to_vaddr(base_address) as *mut u32, fdt_node) };
    SERIAL_PORT.call_once(|| SpinLock::new(pl011));
}

fn lookup_pl011_base_address() -> Option<(usize, FdtNode<'static, 'static>)> {
    let device_tree = DEVICE_TREE.get().unwrap();
    let stdout_path = device_tree
        .find_node("/chosen")?
        .property("stdout-path")?
        .as_str()?;
    let stdout = device_tree.find_node(stdout_path)?;
    if stdout.compatible()?.all().any(|c| c == "arm,pl011") {
        Some((stdout.reg()?.next()?.starting_address as usize, stdout))
    } else {
        None
    }
}
