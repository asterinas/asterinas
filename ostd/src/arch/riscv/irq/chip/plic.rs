// SPDX-License-Identifier: MPL-2.0

//! Platform-Level Interrupt Controller (PLIC) for RISC-V.

use alloc::{boxed::Box, collections::btree_map::BTreeMap, vec::Vec};

use bit_field::BitField;
use fdt::Fdt;

use crate::{
    io::{IoMem, IoMemAllocatorBuilder, Sensitive},
    irq::IrqLine,
    Error, Result,
};

/// The Platform-Level Interrupt Controller (PLIC) for RISC-V.
pub(super) struct Plic {
    phandle: u32,
    io_mem: IoMem<Sensitive>,
    hart_to_target_mapping: BTreeMap<u32, u32>,
    /// Per-Plic interrupt-source-to-IRQ-number mappings.
    interrupt_number_mappings: Box<[Option<u8>]>,
}

impl Plic {
    pub(super) fn phandle(&self) -> u32 {
        self.phandle
    }

    fn num_interrupt_sources(&self) -> u32 {
        self.interrupt_number_mappings.len() as u32
    }

    /// Sets the priority of an interrupt source.
    pub(super) fn set_priority(&mut self, interrupt_source: u32, priority: u32) {
        assert!(interrupt_source > 0 && interrupt_source < self.num_interrupt_sources());

        let offset = Self::PRIORITY_OFFSET + 4 * interrupt_source as usize;
        // SAFETY: The calculation of `offset` follows RISC-V PLIC's
        // specification and is guaranteed to be valid.
        // Reference: <https://github.com/riscv/riscv-plic-spec>.
        unsafe { self.io_mem.write_once(offset, &priority) };
    }

    /// Checks if an interrupt source is pending.
    pub(super) fn is_pending(&self, interrupt_source: u32) -> bool {
        assert!(interrupt_source > 0 && interrupt_source < self.num_interrupt_sources());

        let word_index = interrupt_source as usize / 32;
        let bit_index = interrupt_source as usize % 32;
        let offset = Self::PENDING_OFFSET + 4 * word_index;
        // SAFETY: The calculation of `offset` follows RISC-V PLIC's
        // specification and is guaranteed to be valid.
        // Reference: <https://github.com/riscv/riscv-plic-spec>.
        let value = unsafe { self.io_mem.read_once::<u32>(offset) };
        (value >> bit_index) & 1 != 0
    }

    /// Sets whether an interrupt source is enabled for a specific target (hart).
    pub(super) fn set_interrupt_enabled(&self, hart: u32, interrupt_source: u32, enabled: bool) {
        assert!(interrupt_source > 0 && interrupt_source < self.num_interrupt_sources());

        let target = *self.hart_to_target_mapping.get(&hart).unwrap();
        let word_index = interrupt_source as usize / 32;
        let bit_index = interrupt_source as usize % 32;
        let offset = Self::ENABLE_OFFSET + Self::ENABLE_STRIDE * target as usize + 4 * word_index;

        // SAFETY: The calculation of `offset` follows RISC-V PLIC's
        // specification and is guaranteed to be valid.
        // Reference: <https://github.com/riscv/riscv-plic-spec>.
        let mut value = unsafe { self.io_mem.read_once::<u32>(offset) };
        value.set_bit(bit_index, enabled);
        unsafe { self.io_mem.write_once(offset, &value) };
    }

    /// Sets the threshold for a specific target (hart).
    pub(super) fn set_threshold(&self, hart: u32, threshold: u32) {
        let target = *self.hart_to_target_mapping.get(&hart).unwrap();
        let offset = Self::THRESHOLD_OFFSET + Self::THRESHOLD_STRIDE * target as usize;

        // SAFETY: The calculation of `offset` follows RISC-V PLIC's
        // specification and is guaranteed to be valid.
        // Reference: <https://github.com/riscv/riscv-plic-spec>.
        unsafe { self.io_mem.write_once(offset, &threshold) };
    }

    /// Claims the highest priority pending interrupt for a specific target (hart).
    pub(super) fn claim_interrupt(&self, hart: u32) -> u32 {
        let target = *self.hart_to_target_mapping.get(&hart).unwrap();
        let offset = Self::CLAIM_COMPLETE_OFFSET + Self::CLAIM_COMPLETE_STRIDE * target as usize;

        // SAFETY: The calculation of `offset` follows RISC-V PLIC's
        // specification and is guaranteed to be valid.
        // Reference: <https://github.com/riscv/riscv-plic-spec>.
        unsafe { self.io_mem.read_once::<u32>(offset) }
    }

    /// Completes the interrupt for a specific target (hart) and interrupt source.
    pub(super) fn complete_interrupt(&self, hart: u32, interrupt_source: u32) {
        assert!(interrupt_source > 0 && interrupt_source < self.num_interrupt_sources());

        let target = *self.hart_to_target_mapping.get(&hart).unwrap();
        let offset = Self::CLAIM_COMPLETE_OFFSET + Self::CLAIM_COMPLETE_STRIDE * target as usize;

        // SAFETY: The calculation of `offset` follows RISC-V PLIC's
        // specification and is guaranteed to be valid.
        // Reference: <https://github.com/riscv/riscv-plic-spec>.
        unsafe { self.io_mem.write_once(offset, &interrupt_source) };
    }

    /// Gets an iterator of harts managed by this PLIC.
    pub(super) fn managed_harts(&self) -> impl Iterator<Item = u32> + use<'_> {
        self.hart_to_target_mapping.keys().copied()
    }

    /// Initializes the PLIC.
    pub(super) fn init(&mut self) {
        // Initialize priorities of all interrupt sources to 0.
        for interrupt_source in 1..self.num_interrupt_sources() {
            self.set_priority(interrupt_source, 0);
        }

        for hart in self.managed_harts() {
            // Disable all interrupt sources for all targets.
            for interrupt_source in 1..self.num_interrupt_sources() {
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

    const POSSIBLE_DT_COMPATIBLES: [&str; 4] = [
        "andestech,nceplic100",
        "sifive,plic-1.0.0",
        "thead,c900-plic",
        "riscv,plic0",
    ];
}

impl Plic {
    pub(super) fn from_fdt(fdt: &Fdt<'_>, io_mem_builder: &IoMemAllocatorBuilder) -> Vec<Self> {
        // The parsing logic here assumes a Linux-compatible device tree.
        // Reference: <https://www.kernel.org/doc/Documentation/devicetree/bindings/interrupt-controller/sifive%2Cplic-1.0.0.yaml>.
        fdt.all_nodes()
            .filter(|node| {
                node.compatible().is_some_and(|compatibles| {
                    compatibles
                        .all()
                        .any(|compatible| Self::POSSIBLE_DT_COMPATIBLES.contains(&compatible))
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

                let hart_to_target_mapping = {
                    let cpu_nodes = fdt
                        .find_node("/cpus")
                        .expect("Failed to find '/cpus' node")
                        .children()
                        .filter(|node| node.name.split('@').next().unwrap() == "cpu")
                        .collect::<Vec<_>>();

                    plic_node
                        .property("interrupts-extended")
                        .expect("Failed to read 'interrupts-extended' property from PLIC node")
                        .value
                        .chunks_exact(8)
                        .enumerate()
                        .filter_map(|(idx, chunk)| {
                            let target = [
                                u32::from_be_bytes(chunk[0..4].try_into().unwrap()),
                                u32::from_be_bytes(chunk[4..8].try_into().unwrap()),
                            ];

                            if target[1] != 0x09 {
                                return None;
                            }

                            let hart_id = cpu_nodes.iter().find_map(|node| {
                                node.children()
                                    .find(|child| {
                                        child
                                            .compatible()
                                            .is_some_and(|c| c.all().any(|s| s == "riscv,cpu-intc"))
                                            && child.property("phandle").is_some_and(|ph| {
                                                ph.as_usize().unwrap() as u32 == target[0]
                                            })
                                    })
                                    .and_then(|_| node.property("reg").and_then(|p| p.as_usize()))
                            })?;

                            Some((hart_id as u32, idx as u32))
                        })
                        .collect()
                };

                Self {
                    phandle,
                    io_mem: io_mem_builder.reserve(range, crate::mm::CachePolicy::Uncacheable),
                    hart_to_target_mapping,
                    interrupt_number_mappings: (0..num_interrupt_sources)
                        .map(|_| None)
                        .collect::<Vec<Option<u8>>>()
                        .into_boxed_slice(),
                }
            })
            .collect()
    }

    pub(super) fn interrupt_number_mapping(&self, interrupt_source: u32) -> Option<u8> {
        self.interrupt_number_mappings[interrupt_source as usize]
    }

    pub(super) fn map_interrupt_source_to(
        &mut self,
        interrupt_source: u32,
        irq_line: &IrqLine,
    ) -> Result<()> {
        // An interrupt source number of 0 is reserved to mean “no interrupt”.
        // Reference: <https://github.com/riscv/riscv-plic-spec>.
        assert_ne!(interrupt_source, 0);

        match self.interrupt_number_mappings[interrupt_source as usize] {
            None => {
                self.interrupt_number_mappings[interrupt_source as usize] = Some(irq_line.num());
                Ok(())
            }
            Some(_) => Err(Error::AccessDenied),
        }
    }

    pub(super) fn unmap_interrupt_source(&mut self, interrupt_source: u32) {
        // An interrupt source number of 0 is reserved to mean “no interrupt”.
        // Reference: <https://github.com/riscv/riscv-plic-spec>.
        assert_ne!(interrupt_source, 0);

        self.interrupt_number_mappings[interrupt_source as usize] = None;
    }
}
