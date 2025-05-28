// SPDX-License-Identifier: MPL-2.0

//! Platform-Level Interrupt Controller (PLIC) for RISC-V.

use alloc::{boxed::Box, vec::Vec};

use bit_field::BitField;
use fdt::Fdt;

use crate::{
    io::{IoMem, IoMemAllocatorBuilder, Sensitive},
    trap::irq::IrqLine,
    Error, Result,
};

/// The Platform-Level Interrupt Controller (PLIC) for RISC-V.
pub(super) struct Plic {
    pub(super) phandle: u32,
    io_mem: IoMem<Sensitive>,
    num_interrupt_sources: u32,
    num_targets: u32,
    /// Per-Plic interrupt-source-to-IRQ-number mappings.
    pub(super) interrupt_number_mappings: Box<[Option<u32>]>,
}

impl Plic {
    /// Sets the priority of an interrupt source.
    ///
    /// # Safety
    ///
    /// This function should be called after the kernel page table is activated.
    pub(super) unsafe fn set_priority(&self, interrupt_source: u32, priority: u32) {
        assert!(interrupt_source > 0 && interrupt_source < self.num_interrupt_sources);
        let offset = Self::PRIORITY_OFFSET + 4 * interrupt_source as usize;
        // SAFETY:
        // 1. The caller ensures that the kernel page table is activated.
        // 2. The calculation of `offset` follows RISC-V PLIC's
        //    specification and is guaranteed to be valid. See
        //    https://github.com/riscv/riscv-plic-spec.
        unsafe {
            self.io_mem.write_once(offset, &priority).unwrap();
        }
    }

    /// Checks if an interrupt source is pending.
    ///
    /// # Safety
    ///
    /// This function should be called after the kernel page table is activated.
    pub(super) unsafe fn is_pending(&self, interrupt_source: u32) -> bool {
        assert!(interrupt_source > 0 && interrupt_source < self.num_interrupt_sources);
        let word_index = interrupt_source as usize / 32;
        let bit_index = interrupt_source as usize % 32;
        let offset = Plic::PENDING_OFFSET + 4 * word_index;
        // SAFETY:
        // 1. The caller ensures that the kernel page table is activated.
        // 2. The calculation of `offset` follows RISC-V PLIC's
        //    specification and is guaranteed to be valid. See
        //    https://github.com/riscv/riscv-plic-spec.
        let value = unsafe { self.io_mem.read_once::<u32>(offset).unwrap() };
        (value >> bit_index) & 1 != 0
    }

    /// Sets whether an interrupt source is enabled for a specific target (hart).
    ///
    /// # Safety
    ///
    /// This function should be called after the kernel page table is activated.
    pub(super) unsafe fn set_interrupt_enabled(
        &self,
        hart: u32,
        interrupt_source: u32,
        enabled: bool,
    ) {
        assert!(interrupt_source > 0 && interrupt_source < self.num_interrupt_sources);
        let target = hart * 2 + 1;
        assert!(target < self.num_targets);
        let word_index = interrupt_source as usize / 32;
        let bit_index = interrupt_source as usize % 32;
        let offset = Plic::ENABLE_OFFSET + Plic::ENABLE_STRIDE * target as usize + 4 * word_index;
        let mut value = unsafe { self.io_mem.read_once::<u32>(offset).unwrap() };
        value.set_bit(bit_index, enabled);
        // SAFETY:
        // 1. The caller ensures that the kernel page table is activated.
        // 2. The calculation of `offset` follows RISC-V PLIC's
        //    specification and is guaranteed to be valid. See
        //    https://github.com/riscv/riscv-plic-spec.
        unsafe {
            self.io_mem.write_once(offset, &value).unwrap();
        }
    }

    /// Sets the threshold for a specific target (hart).
    ///
    /// # Safety
    ///
    /// This function should be called after the kernel page table is activated.
    pub(super) unsafe fn set_threshold(&self, hart: u32, threshold: u32) {
        let target = hart * 2 + 1;
        assert!(target < self.num_targets);
        let offset = Plic::THRESHOLD_OFFSET + Plic::THRESHOLD_STRIDE * target as usize;
        // SAFETY:
        // 1. The caller ensures that the kernel page table is activated.
        // 2. The calculation of `offset` follows RISC-V PLIC's
        //    specification and is guaranteed to be valid. See
        //    https://github.com/riscv/riscv-plic-spec.
        unsafe {
            self.io_mem.write_once(offset, &threshold).unwrap();
        }
    }

    /// Claims the highest priority pending interrupt for a specific target (hart).
    ///
    /// # Safety
    ///
    /// This function should be called after the kernel page table is activated.
    pub(super) unsafe fn claim_interrupt(&self, hart: u32) -> u32 {
        let target = hart * 2 + 1;
        assert!(target < self.num_targets);
        let offset = Plic::CLAIM_COMPLETE_OFFSET + Plic::CLAIM_COMPLETE_STRIDE * target as usize;
        // SAFETY:
        // 1. The caller ensures that the kernel page table is activated.
        // 2. The calculation of `offset` follows RISC-V PLIC's
        //    specification and is guaranteed to be valid. See
        //    https://github.com/riscv/riscv-plic-spec.
        unsafe { self.io_mem.read_once::<u32>(offset).unwrap() }
    }

    /// Completes the interrupt for a specific target (hart) and interrupt source.
    ///
    /// # Safety
    ///
    /// This function should be called after the kernel page table is activated.
    pub(super) unsafe fn complete_interrupt(&self, hart: u32, interrupt_source: u32) {
        assert!(interrupt_source > 0 && interrupt_source < self.num_interrupt_sources);
        let target = hart * 2 + 1;
        assert!(target < self.num_targets);
        let offset = Plic::CLAIM_COMPLETE_OFFSET + Plic::CLAIM_COMPLETE_STRIDE * target as usize;
        // SAFETY:
        // 1. The caller ensures that the kernel page table is activated.
        // 2. The calculation of `offset` follows RISC-V PLIC's
        //    specification and is guaranteed to be valid. See
        //    https://github.com/riscv/riscv-plic-spec.
        unsafe {
            self.io_mem.write_once(offset, &interrupt_source).unwrap();
        }
    }

    /// Initializes the PLIC.
    ///
    /// # Safety
    ///
    /// This function should be called after the kernel page table is activated.
    pub(super) unsafe fn init(&self) {
        // SAFETY: The caller ensures that the kernel page table is activated.
        unsafe {
            // Initialize priorities of all interrupt sources to 0.
            for interrupt_source in 1..self.num_interrupt_sources {
                self.set_priority(interrupt_source, 0);
            }

            assert!(self.num_targets % 2 == 0);
            for hart in 0..(self.num_targets / 2) {
                // Disable all interrupt sources for all targets.
                for interrupt_source in 1..self.num_interrupt_sources {
                    self.set_interrupt_enabled(hart, interrupt_source, false);
                }

                // Set all targets' thresholds to 0 to allow all priority levels.
                self.set_threshold(hart, 0);

                // Clear all pending claims.
                while let irq_num = self.claim_interrupt(hart)
                    && irq_num != 0
                {
                    self.complete_interrupt(hart, irq_num);
                }
            }
        }
    }
}

impl Plic {
    // Here we define the constants for PLIC MMIO access.
    //
    // The layout of PLIC MMIO region is as follows
    // +-------------------------------------------------------------------+
    // |                         PLIC MMIO Region                          |
    // | (Base Address: e.g., 0x0C00_0000)                                 |
    // +-------------------------------------------------------------------+
    // |                                                                   |
    // |  +-------------------------------------------------------------+  |
    // |  |             Interrupt Source Priority Registers             |  |
    // |  | (Offset: 0x0000_0000)                                       |  |
    // |  |                                                             |  |
    // |  | - 32-bit register per interrupt source (Source 1 to N)      |  |
    // |  | - Offset for Source I: 0x0 + (I * 4)                        |  |
    // |  | - Used to set priority (0 = disabled, higher value = higher)|  |
    // |  +-------------------------------------------------------------+  |
    // |                                                                   |
    // |  +-------------------------------------------------------------+  |
    // |  |                 Interrupt Pending Registers                 |  |
    // |  | (Offset: 0x0000_1000)                                       |  |
    // |  |                                                             |  |
    // |  | - 32-bit registers, bit-mapped for pending interrupts       |  |
    // |  | - Word Index: Source ID / 32                                |  |
    // |  | - Bit Index: Source ID % 32                                 |  |
    // |  | - Read-only to check if an interrupt is pending             |  |
    // |  +-------------------------------------------------------------+  |
    // |                                                                   |
    // |  +-------------------------------------------------------------+  |
    // |  |             Interrupt Enable Registers (per Target)         |  |
    // |  | (Offset: 0x0000_2000)                                       |  |
    // |  |                                                             |  |
    // |  | +---------------------------------------------------------+ |  |
    // |  | | Target 0 Enable Bits (Offset: 0x0000_2000)              | |  |
    // |  | | - 32-bit registers, bit-mapped for interrupt enables    | |  |
    // |  | | - Word Index: Source ID / 32                            | |  |
    // |  | | - Bit Index: Source ID % 32                             | |  |
    // |  | +---------------------------------------------------------+ |  |
    // |  | | Target 1 Enable Bits (Offset: 0x0000_2000 + 0x80)       | |  |
    // |  | | ...                                                     | |  |
    // |  | | Target M Enable Bits (Offset: 0x0000_2000 + M*0x80)     | |  |
    // |  | +---------------------------------------------------------+ |  |
    // |  | - Used to enable/disable specific interrupts for each hart  |  |
    // |  +-------------------------------------------------------------+  |
    // |                                                                   |
    // |  +-------------------------------------------------------------+  |
    // |  |             Priority Threshold Registers (per Target)       |  |
    // |  | (Offset: 0x0020_0000)                                       |  |
    // |  |                                                             |  |
    // |  | +---------------------------------------------------------+ |  |
    // |  | | Target 0 Threshold (Offset: 0x0020_0000)                | |  |
    // |  | | - 32-bit register                                       | |  |
    // |  | +---------------------------------------------------------+ |  |
    // |  | | Target 1 Threshold (Offset: 0x0020_0000 + 0x1000)       | |  |
    // |  | | ...                                                     | |  |
    // |  | | Target M Threshold (Offset: 0x0020_0000 + M*0x1000)     | |  |
    // |  | +---------------------------------------------------------+ |  |
    // |  | - Used to set the minimum priority for interrupts to be     |  |
    // |  |   delivered to a target.                                    |  |
    // |  +-------------------------------------------------------------+  |
    // |                                                                   |
    // |  +-------------------------------------------------------------+  |
    // |  |          Claim/Complete Registers (per Target)              |  |
    // |  | (Offset: 0x0020_0004)                                       |  |
    // |  |                                                             |  |
    // |  | +---------------------------------------------------------+ |  |
    // |  | | Target 0 Claim/Complete (Offset: 0x0020_0004)           | |  |
    // |  | | - 32-bit register                                       | |  |
    // |  | +---------------------------------------------------------+ |  |
    // |  | | Target 1 Claim/Complete (Offset: 0x0020_0004 + 0x1000)  | |  |
    // |  | | ...                                                     | |  |
    // |  | | Target M Claim/Complete (Offset: 0x0020_0004 + M*0x1000)| |  |
    // |  | +---------------------------------------------------------+ |  |
    // |  | - Read to claim the highest priority pending interrupt ID.  |  |
    // |  | - Write the ID back to complete the interrupt.              |  |
    // |  +-------------------------------------------------------------+  |
    // |                                                                   |
    // +-------------------------------------------------------------------+
    const PRIORITY_OFFSET: usize = 0x0;
    const PENDING_OFFSET: usize = 0x1000;
    const ENABLE_OFFSET: usize = 0x2000;
    const THRESHOLD_OFFSET: usize = 0x200000;
    const CLAIM_COMPLETE_OFFSET: usize = 0x200004;

    const ENABLE_STRIDE: usize = 0x80;
    const THRESHOLD_STRIDE: usize = 0x1000;
    const CLAIM_COMPLETE_STRIDE: usize = 0x1000;
}

impl Plic {
    pub(super) fn from_fdt(fdt: &Fdt<'_>, io_mem_builder: &IoMemAllocatorBuilder) -> Vec<Self> {
        fdt.all_nodes()
            .filter(|node| {
                let possible_compatibles = [
                    "andestech,nceplic100",
                    "sifive,plic-1.0.0",
                    "thead,c900-plic",
                    "riscv,plic0",
                ];
                node.compatible().is_some_and(|compatibles| {
                    compatibles
                        .all()
                        .any(|compatible| possible_compatibles.contains(&compatible))
                })
            })
            .map(|plic_node| {
                let phandle = plic_node
                    .property("phandle")
                    .and_then(|prop| prop.as_usize())
                    .expect("Failed to read 'phandle' property from PLIC node")
                    as u32;
                let range = {
                    let region = plic_node
                        .reg()
                        .expect("Failed to read 'reg' property from PLIC node")
                        .next()
                        .expect("Empty 'reg' property found in PLIC node");
                    let base = region.starting_address as usize;
                    let size = region
                        .size
                        .expect("Incomplete 'reg' property found in PLIC node");
                    base..(base + size)
                };
                let num_interrupt_sources = plic_node
                    .property("riscv,ndev")
                    .and_then(|prop| prop.as_usize())
                    .expect("Failed to read 'riscv,ndev' property from PLIC node")
                    as u32;
                let num_targets = plic_node
                    .property("interrupts-extended")
                    .map(|prop| prop.value.len() / 8)
                    .expect("Failed to read 'interrupts-extended' property from PLIC node")
                    as u32;
                Self {
                    phandle,
                    io_mem: io_mem_builder.reserve_io_mem(range),
                    num_interrupt_sources,
                    num_targets,
                    interrupt_number_mappings: (0..num_interrupt_sources)
                        .map(|_| None)
                        .collect::<Vec<Option<u32>>>()
                        .into_boxed_slice(),
                }
            })
            .collect()
    }

    pub(super) fn map_interrupt_source_to(
        &mut self,
        interrupt_source: u32,
        irq_line: &IrqLine,
    ) -> Result<()> {
        assert!(interrupt_source > 0 && interrupt_source < self.num_interrupt_sources);
        match self.interrupt_number_mappings[interrupt_source as usize] {
            None => {
                self.interrupt_number_mappings[interrupt_source as usize] =
                    Some(irq_line.num() as u32);
                Ok(())
            }
            Some(_) => Err(Error::InvalidArgs),
        }
    }

    pub(super) fn unmap_interrupt_source(&mut self, interrupt_source: u32) {
        assert!(interrupt_source > 0 && interrupt_source < self.num_interrupt_sources);
        self.interrupt_number_mappings[interrupt_source as usize] = None;
    }
}
