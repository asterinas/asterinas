// SPDX-License-Identifier: MPL-2.0

use bit_field::BitField;
use fdt::Fdt;
use spin::Once;

use crate::{
    arch::boot::DEVICE_TREE,
    io::{IoMem, IoMemAllocatorBuilder},
    mm::{CachePolicy, PageFlags, VmIoOnce},
};

/// Initializes the Platform-Level Interrupt Controller (PLIC).
///
/// # Safety
///
/// This function is safe to call on the following conditions:
/// 1. It is called once and at most once at a proper timing in the boot context.
/// 2. It is called before any other public functions of this module is called.
pub(crate) unsafe fn init(io_mem_builder: &IoMemAllocatorBuilder) {
    let device_tree = DEVICE_TREE.get().unwrap();
    let plic_builder = PlicBuilder::from_fdt(device_tree);
    PLIC.call_once(|| plic_builder.build(io_mem_builder));
}

/// Resets PLIC states.
///
/// # Safety
///
/// This function is safe to call on the following conditions:
/// 1. It is called only once in the boot context after the kernel page table is
///    activated.
/// 2. It is called before any other public functions (except the `init`) of
///    this module is called.
pub(crate) unsafe fn init_after_kpt() {
    PLIC.get().unwrap().init();
}

pub(crate) fn claim_interrupt(target: usize) -> usize {
    PLIC.get().unwrap().claim_interrupt(target) as usize
}

pub(crate) fn complete_interrupt(target: usize, interrupt_source: usize) {
    PLIC.get()
        .unwrap()
        .complete_interrupt(target, interrupt_source);
}

static PLIC: Once<Plic> = Once::new();

/// The Platform-Level Interrupt Controller (PLIC) for RISC-V.
struct Plic {
    io_mem: IoMem,
    num_interrupt_sources: usize,
    num_targets: usize,
}

impl Plic {
    pub fn set_priority(&self, interrupt_source: usize, priority: u32) {
        debug_assert!(interrupt_source <= self.num_interrupt_sources);
        let offset = Self::PRIORITY_OFFSET + 4 * interrupt_source;
        self.io_mem.write_once(offset, &priority).unwrap();
    }

    #[allow(unused)]
    pub fn is_pending(&self, interrupt_source: usize) -> bool {
        debug_assert!(interrupt_source <= self.num_interrupt_sources);
        let word_index = interrupt_source / 32;
        let bit_index = interrupt_source % 32;
        let offset = Plic::PENDING_OFFSET + 4 * word_index;
        let value = self.io_mem.read_once::<u32>(offset).unwrap();
        (value >> bit_index) & 1 != 0
    }

    pub fn set_interrupt_enabled(&self, target: usize, interrupt_source: usize, enabled: bool) {
        debug_assert!(target < self.num_targets && interrupt_source < self.num_interrupt_sources);
        let word_index = interrupt_source / 32;
        let bit_index = interrupt_source % 32;
        let offset = Plic::ENABLE_OFFSET + Plic::ENABLE_STRIDE * target + 4 * word_index;
        let mut value = self.io_mem.read_once::<u32>(offset).unwrap();
        value.set_bit(bit_index, enabled);
        self.io_mem.write_once(offset, &value).unwrap();
    }

    pub fn set_threshold(&self, target: usize, threshold: u32) {
        debug_assert!(target < self.num_targets);
        let offset = Plic::THRESHOLD_OFFSET + Plic::THRESHOLD_STRIDE * target;
        self.io_mem.write_once(offset, &threshold).unwrap();
    }

    pub fn claim_interrupt(&self, target: usize) -> usize {
        debug_assert!(target < self.num_targets);
        let offset = Plic::CLAIM_COMPLETE_OFFSET + Plic::CLAIM_COMPLETE_STRIDE * target;
        self.io_mem.read_once::<u32>(offset).unwrap() as usize
    }

    pub fn complete_interrupt(&self, target: usize, interrupt_source: usize) {
        debug_assert!(target < self.num_targets);
        let offset = Plic::CLAIM_COMPLETE_OFFSET + Plic::CLAIM_COMPLETE_STRIDE * target;
        self.io_mem
            .write_once(offset, &(interrupt_source as u32))
            .unwrap();
    }

    fn init(&self) {
        // Initialize all priorities to 1.
        for interrupt_source in 0..self.num_interrupt_sources {
            self.set_priority(interrupt_source, 1);
        }

        // Disable all interrupts for all targets.
        for target in 0..(self.num_targets) {
            for interrupt_source in 0..self.num_interrupt_sources {
                self.set_interrupt_enabled(target, interrupt_source, false);
            }
        }

        // Set all targets' thresholds to 0 to allow all priority levels.
        for target in 0..self.num_targets {
            self.set_threshold(target, 0);
        }

        // Clear all pending claims.
        for target in 0..self.num_targets {
            while let irq_num = self.claim_interrupt(target)
                && irq_num != 0
            {
                self.complete_interrupt(target, irq_num);
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

struct PlicBuilder {
    base_address: usize,
    num_interrupt_sources: usize,
    num_targets: usize,
}

impl PlicBuilder {
    fn from_fdt(fdt: &Fdt<'_>) -> Self {
        let plic_node = {
            let possible_compatibles = [
                "andestech,nceplic100",
                "sifive,plic-1.0.0",
                "thead,c900-plic",
                "riscv,plic0",
            ];
            fdt.find_compatible(&possible_compatibles)
                .expect("Failed to find PLIC node in device tree")
        };
        let base_address = plic_node.reg().unwrap().next().unwrap().starting_address as usize;
        let num_interrupt_sources = plic_node
            .property("riscv,ndev")
            .and_then(|prop| prop.as_usize())
            .expect("Failed to read 'riscv,ndev' property from PLIC node");
        let num_targets = plic_node
            .property("interrupts-extended")
            .and_then(|prop| Some(prop.value.len() / 8))
            .expect("Failed to read 'interrupts-extended' property from PLIC node");
        Self {
            base_address,
            num_interrupt_sources,
            num_targets,
        }
    }

    fn build(self, io_mem_builder: &IoMemAllocatorBuilder) -> Plic {
        let size = Plic::THRESHOLD_OFFSET + self.num_targets * Plic::THRESHOLD_STRIDE;
        io_mem_builder.remove(self.base_address..self.base_address + size);

        Plic {
            // SAFETY: We are building I/O memory using a region that is
            // specified as PLIC I/O memory in device tree.
            io_mem: unsafe {
                IoMem::new(
                    self.base_address..self.base_address + size,
                    PageFlags::RW,
                    CachePolicy::Uncacheable,
                )
            },
            num_interrupt_sources: self.num_interrupt_sources,
            num_targets: self.num_targets,
        }
    }
}
