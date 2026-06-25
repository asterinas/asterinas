// SPDX-License-Identifier: MPL-2.0

//! Incoming Message-Signaled Interrupt Controller (IMSIC).

use alloc::{boxed::Box, vec::Vec};
use core::arch::asm;

use fdt::Fdt;

use crate::io::{IoMem, IoMemAllocatorBuilder, Sensitive};

/// A supervisor-level IMSIC interrupt-file group.
pub(super) struct Imsic {
    phandle: u32,
    message_address: usize,
    num_ids: u32,
    _regions: Box<[IoMem<Sensitive>]>,
}

impl Imsic {
    // Register offsets follow the RISC-V AIA specification's IMSIC
    // register layout.
    const EIDELIVERY: usize = 0x70;
    const EITHRESHOLD: usize = 0x72;
    const EIP0: usize = 0x80;
    const EIE0: usize = 0xc0;

    /// Discovers the supervisor-level IMSIC from the device tree.
    pub(super) fn from_fdt(fdt: &Fdt<'_>, io_mem_builder: &IoMemAllocatorBuilder) -> Option<Self> {
        let node = fdt.all_nodes().find(|node| {
            let compatible = node.compatible().is_some_and(|compatibles| {
                compatibles
                    .all()
                    .any(|compatible| compatible == "riscv,imsics")
            });
            let supervisor_interrupt =
                node.property("interrupts-extended")
                    .is_some_and(|property| {
                        property
                            .value
                            .chunks_exact(8)
                            .any(|entry| u32::from_be_bytes(entry[4..8].try_into().unwrap()) == 9)
                    });
            compatible && supervisor_interrupt
        })?;

        let phandle = node.property("phandle")?.as_usize()? as u32;
        let num_ids = node.property("riscv,num-ids")?.as_usize()? as u32;
        let mut message_address = None;
        let regions = node
            .reg()?
            .map(|region| {
                let start = region.starting_address as usize;
                let size = region.size.expect("Incomplete IMSIC 'reg' property");
                let end = start.checked_add(size).expect("IMSIC MMIO range overflows");
                message_address.get_or_insert(start);
                io_mem_builder.reserve(start..end, crate::mm::CachePolicy::Uncacheable)
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();

        Some(Self {
            phandle,
            message_address: message_address?,
            num_ids,
            _regions: regions,
        })
    }

    pub(super) fn phandle(&self) -> u32 {
        self.phandle
    }

    pub(super) fn message_address(&self) -> usize {
        self.message_address
    }

    /// Initializes the current hart's supervisor interrupt file.
    pub(super) fn init_current_hart(&self) {
        Self::write_indirect(Self::EIDELIVERY, 0);
        Self::write_indirect(Self::EITHRESHOLD, 0);

        let register_count = (self.num_ids as usize).div_ceil(64);
        for register_index in 0..register_count {
            let selector_offset = register_index * 2;
            Self::write_indirect(Self::EIE0 + selector_offset, 0);
            Self::write_indirect(Self::EIP0 + selector_offset, 0);
        }

        Self::write_indirect(Self::EIDELIVERY, 1);
    }

    /// Enables an interrupt identity in the current hart's interrupt file.
    pub(super) fn enable(&self, interrupt_id: u16) -> bool {
        if !self.is_valid_interrupt_id(interrupt_id) {
            return false;
        }

        let interrupt_id = interrupt_id as usize;
        let register_index = interrupt_id / 64;
        let bit_index = interrupt_id % 64;
        let selector = Self::EIE0 + register_index * 2;
        let enabled = Self::read_indirect(selector) | 1usize << bit_index;
        Self::write_indirect(selector, enabled);
        true
    }

    /// Disables an interrupt identity in the current hart's interrupt file.
    pub(super) fn disable(&self, interrupt_id: u16) {
        if !self.is_valid_interrupt_id(interrupt_id) {
            return;
        }

        let interrupt_id = interrupt_id as usize;
        let register_index = interrupt_id / 64;
        let bit_index = interrupt_id % 64;
        let selector = Self::EIE0 + register_index * 2;
        let enabled = Self::read_indirect(selector) & !(1usize << bit_index);
        Self::write_indirect(selector, enabled);
    }

    /// Claims the highest-priority pending interrupt identity.
    pub(super) fn claim(&self) -> Option<u16> {
        let value: usize;
        // SAFETY: This driver is instantiated only when the device tree exposes
        // a supervisor-level IMSIC, making `stopei` a valid CSR.
        unsafe {
            asm!("csrrw {value}, 0x15c, zero", value = out(reg) value, options(nostack));
        }
        let interrupt_id = ((value >> 16) & 0x7ff) as u16;
        self.is_valid_interrupt_id(interrupt_id)
            .then_some(interrupt_id)
    }

    fn is_valid_interrupt_id(&self, interrupt_id: u16) -> bool {
        interrupt_id != 0 && u32::from(interrupt_id) < self.num_ids
    }

    fn read_indirect(selector: usize) -> usize {
        let value: usize;
        // SAFETY: The selected register belongs to the supervisor IMSIC
        // interrupt file discovered from the device tree.
        unsafe {
            asm!("csrw 0x150, {selector}", selector = in(reg) selector, options(nostack));
            asm!("csrr {value}, 0x151", value = out(reg) value, options(nostack));
        }
        value
    }

    fn write_indirect(selector: usize, value: usize) {
        // SAFETY: The selected register belongs to the supervisor IMSIC
        // interrupt file discovered from the device tree.
        unsafe {
            asm!("csrw 0x150, {selector}", selector = in(reg) selector, options(nostack));
            asm!("csrw 0x151, {value}", value = in(reg) value, options(nostack));
        }
    }
}
