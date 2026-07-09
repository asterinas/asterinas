// SPDX-License-Identifier: MPL-2.0

//! The console I/O backed by an ARM PL011 UART.
//!
//! The QEMU `virt` machine places a PL011 at physical address `0x0900_0000`.
//! It is reachable through the kernel's linear mapping from the very start of
//! the Rust boot path.
//!
//! TODO: Probe the UART base address and interrupt from the device tree, and
//! map it as Device memory through an [`crate::io::IoMem`] region instead of
//! relying on the linear mapping.

use core::fmt;

use spin::Once;

use crate::{
    boot::EarlyCmdline,
    mm::paddr_to_vaddr,
    sync::{LocalIrqDisabled, SpinLock},
};

/// Physical base address of the PL011 UART on the QEMU `virt` machine.
const PL011_BASE_PADDR: usize = 0x0900_0000;

/// Data register (offset 0x00).
const UART_DR: usize = 0x00;
/// Flag register (offset 0x18).
const UART_FR: usize = 0x18;
/// Line control register (offset 0x2C).
const UART_LCRH: usize = 0x2c;
/// Control register (offset 0x30).
const UART_CR: usize = 0x30;
/// Receive FIFO empty (FR bit 4).
const UART_FR_RXFE: u32 = 1 << 4;
/// Transmit FIFO full (FR bit 5).
const UART_FR_TXFF: u32 = 1 << 5;
/// UART enable (CR bit 0).
const UART_CR_UARTEN: u32 = 1 << 0;
/// Transmit enable (CR bit 8).
const UART_CR_TXE: u32 = 1 << 8;
/// Receive enable (CR bit 9).
const UART_CR_RXE: u32 = 1 << 9;
/// Word length 8 bits (LCR_H bits 6:5).
const UART_LCRH_WLEN_8: u32 = 0b11 << 5;
/// Enable FIFOs (LCR_H bit 4).
const UART_LCRH_FEN: u32 = 1 << 4;

/// The primary serial port, which serves as an early console.
pub static SERIAL_PORT: Once<SpinLock<Pl011, LocalIrqDisabled>> =
    Once::initialized(SpinLock::new(Pl011::new()));

/// An ARM PL011 UART used as the early console.
pub struct Pl011 {
    _private: (),
}

impl Pl011 {
    const fn new() -> Self {
        Self { _private: () }
    }

    fn reg(&self, offset: usize) -> *mut u32 {
        (paddr_to_vaddr(PL011_BASE_PADDR) + offset) as *mut u32
    }

    /// Ensures the UART is enabled for transmission. Idempotent, so it is safe
    /// to call on every early write before [`init`] runs.
    fn ensure_enabled(&self) {
        // SAFETY: The PL011 registers are mapped through the linear mapping.
        unsafe {
            if core::ptr::read_volatile(self.reg(UART_CR)) & UART_CR_UARTEN != 0 {
                return;
            }
            core::ptr::write_volatile(self.reg(UART_CR), 0);
            core::ptr::write_volatile(self.reg(UART_LCRH), UART_LCRH_WLEN_8 | UART_LCRH_FEN);
            core::ptr::write_volatile(
                self.reg(UART_CR),
                UART_CR_UARTEN | UART_CR_TXE | UART_CR_RXE,
            );
        }
    }

    /// Sends a single byte, blocking until the transmit FIFO has room.
    pub fn send(&self, byte: u8) {
        self.ensure_enabled();
        // SAFETY: The PL011 registers are mapped through the linear mapping and
        // accessed with volatile operations.
        unsafe {
            while core::ptr::read_volatile(self.reg(UART_FR)) & UART_FR_TXFF != 0 {
                core::hint::spin_loop();
            }
            core::ptr::write_volatile(self.reg(UART_DR), byte as u32);
        }
    }

    /// Receives a single byte if one is available in the receive FIFO.
    pub fn recv(&self) -> Option<u8> {
        // SAFETY: The PL011 registers are mapped through the linear mapping.
        unsafe {
            if core::ptr::read_volatile(self.reg(UART_FR)) & UART_FR_RXFE != 0 {
                None
            } else {
                Some(core::ptr::read_volatile(self.reg(UART_DR)) as u8)
            }
        }
    }
}

impl fmt::Write for Pl011 {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &byte in s.as_bytes() {
            self.send(byte);
        }
        Ok(())
    }
}

/// Initializes the serial port.
///
/// QEMU pre-initialises the PL011 into a usable state, so no register
/// programming is required to emit early output.
pub(crate) fn init(_early_cmdline: &EarlyCmdline) {}
