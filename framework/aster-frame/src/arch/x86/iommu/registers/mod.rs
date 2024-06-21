// SPDX-License-Identifier: MPL-2.0

//! Registers and their definition used by IOMMU.

#![allow(unused_imports)]
#![allow(dead_code)]
#![allow(unused_variables)]

mod capability;
mod command;
mod extended_cap;
mod invalidation;
mod status;

pub use capability::*;
use command::GlobalCommand;
pub use command::*;
pub use extended_cap::*;
use invalidation::InvalidationRegisters;
use log::{debug, info};
use spin::Once;
use status::GlobalStatus;
pub use status::*;
use volatile::{
    access::{ReadOnly, ReadWrite, WriteOnly},
    Volatile,
};
use x86_64::instructions::interrupts::enable_and_hlt;

use super::{
    dma_remapping::context_table::RootTable, interrupt_remapping::IntRemappingTable,
    invalidate::queue::Queue, IommuError,
};
use crate::{
    arch::{
        iommu::{
            fault,
            invalidate::{descriptor::InterruptEntryCache, QUEUE},
        },
        x86::kernel::acpi::{
            dmar::{Dmar, Remapping},
            ACPI_TABLES,
        },
    },
    mm::paddr_to_vaddr,
    prelude::Paddr,
    sync::SpinLock,
};

/// Important registers used by IOMMU.
#[derive(Debug)]
pub struct IommuRegisters {
    version: Volatile<&'static u32, ReadOnly>,
    capability: Volatile<&'static u64, ReadOnly>,
    extended_capability: Volatile<&'static u64, ReadOnly>,
    global_command: Volatile<&'static mut u32, WriteOnly>,
    global_status: Volatile<&'static u32, ReadOnly>,
    root_table_address: Volatile<&'static mut u64, ReadWrite>,
    context_command: Volatile<&'static mut u64, ReadWrite>,

    interrupt_remapping_table_addr: Volatile<&'static mut u64, ReadWrite>,

    invalidate: InvalidationRegisters,
}

impl IommuRegisters {
    /// Capability of IOMMU
    pub fn capability(&self) -> Capability {
        Capability::new(self.capability.read())
    }

    /// Extended Capability of IOMMU
    pub fn extended_capability(&self) -> ExtendedCapability {
        ExtendedCapability::new(self.extended_capability.read())
    }

    /// Global Status of IOMMU
    pub fn global_status(&self) -> GlobalStatus {
        GlobalStatus::from_bits_truncate(self.global_status.read())
    }

    /// Enable DMA remapping with static RootTable
    pub(super) fn enable_dma_remapping(&mut self, root_table: &'static SpinLock<RootTable>) {
        // Set root table address
        self.root_table_address
            .write(root_table.lock().paddr() as u64);
        self.write_global_command(GlobalCommand::SRTP, true);
        while !self.global_status().contains(GlobalStatus::RTPS) {}

        // Enable DMA remapping
        self.write_global_command(GlobalCommand::TE, true);
        while !self.global_status().contains(GlobalStatus::TES) {}
    }

    /// Enable Interrupt Remapping with IntRemappingTable
    pub(super) fn enable_interrupt_remapping(&mut self, table: &'static IntRemappingTable) {
        assert!(self
            .extended_capability()
            .flags()
            .contains(ExtendedCapabilityFlags::IR));
        // Set interrupt remapping table address
        self.interrupt_remapping_table_addr.write(table.encode());
        self.write_global_command(GlobalCommand::SIRTP, true);
        while !self.global_status().contains(GlobalStatus::IRTPS) {}

        // Enable Interrupt Remapping
        self.write_global_command(GlobalCommand::IRE, true);
        while !self.global_status().contains(GlobalStatus::IRES) {}

        // Invalidate interrupt cache
        if self.global_status().contains(GlobalStatus::QIES) {
            let mut queue = QUEUE.get().unwrap().lock_irq_disabled();
            queue.append_descriptor(InterruptEntryCache::global_invalidation().0);
            let tail = queue.tail();
            self.invalidate.queue_tail.write((tail << 4) as u64);
            while (self.invalidate.queue_head.read() >> 4) == tail as u64 - 1 {}

            queue.append_descriptor(0x5 | 0x10);
            let tail = queue.tail();
            self.invalidate.queue_tail.write((tail << 4) as u64);
            while self.invalidate.completion_status.read() == 0 {}
        } else {
            self.global_invalidation()
        }

        // Disable Compatibility format interrupts
        if self.global_status().contains(GlobalStatus::CFIS) {
            self.write_global_command(GlobalCommand::CFI, false);
            while self.global_status().contains(GlobalStatus::CFIS) {}
        }
    }

    pub(super) fn enable_queued_invalidation(&mut self, queue: &Queue) {
        assert!(self
            .extended_capability()
            .flags()
            .contains(ExtendedCapabilityFlags::QI));
        self.invalidate.queue_tail.write(0);

        let mut write_value = queue.base_paddr() as u64;
        // By default, we set descriptor width to 128-bit(0)
        let descriptor_width = 0b0;
        write_value |= descriptor_width << 11;

        let mut queue_size = queue.size();
        assert!(queue_size.is_power_of_two());
        let mut size = 0;
        if descriptor_width == 0 {
            // 2^(X + 8) = number of entries
            assert!(queue_size >= (1 << 8));
            queue_size >>= 8;
        } else {
            // 2^(X + 7) = number of entries
            assert!(queue_size >= (1 << 7));
            queue_size >>= 7;
        };
        while queue_size & 0b1 == 0 {
            queue_size >>= 1;
            size += 1;
        }
        write_value |= size;

        self.invalidate.queue_addr.write(write_value);

        // Enable Queued invalidation
        self.write_global_command(GlobalCommand::QIE, true);
        while !self.global_status().contains(GlobalStatus::QIES) {}
    }

    fn global_invalidation(&mut self) {
        self.context_command.write(0xA000_0000_0000_0000);
        let mut value = 0x8000_0000_0000_0000;
        while (value & 0x8000_0000_0000_0000) != 0 {
            value = self.context_command.read();
        }

        self.invalidate
            .iotlb_invalidate
            .write(0x9000_0000_0000_0000);
    }

    /// Write value to the global command register. This function will not wait until the command
    /// is serviced. User need to check the global status register.
    fn write_global_command(&mut self, command: GlobalCommand, enable: bool) {
        const ONE_SHOT_STATUS_MASK: u32 = 0x96FF_FFFF;
        let status = self.global_status.read() & ONE_SHOT_STATUS_MASK;
        if enable {
            self.global_command.write(status | command.bits());
        } else {
            self.global_command.write(status & !command.bits());
        }
    }

    /// Create an instance from base address
    fn new() -> Option<Self> {
        let dmar = Dmar::new()?;
        let acpi_table_lock = ACPI_TABLES.get().unwrap().lock();

        debug!("DMAR:{:#x?}", dmar);
        let base_address = {
            let mut addr = 0;
            for remapping in dmar.remapping_iter() {
                if let Remapping::Drhd(drhd) = remapping {
                    addr = drhd.register_base_addr()
                }
            }
            if addr == 0 {
                panic!("There should be a DRHD structure in the DMAR table");
            }
            addr
        };

        let vaddr: usize = paddr_to_vaddr(base_address as usize);
        // SAFETY: All offsets and sizes are strictly adhered to in the manual, and the base address is obtained from Drhd.
        let iommu_regs = unsafe {
            fault::init(vaddr);
            let version = Volatile::new_read_only(&*(vaddr as *const u32));
            let capability = Volatile::new_read_only(&*((vaddr + 0x08) as *const u64));
            let extended_capability: Volatile<&u64, ReadOnly> =
                Volatile::new_read_only(&*((vaddr + 0x10) as *const u64));
            let global_command = Volatile::new_write_only(&mut *((vaddr + 0x18) as *mut u32));
            let global_status = Volatile::new_read_only(&*((vaddr + 0x1C) as *const u32));
            let root_table_address = Volatile::new(&mut *((vaddr + 0x20) as *mut u64));
            let context_command = Volatile::new(&mut *((vaddr + 0x28) as *mut u64));

            let interrupt_remapping_table_addr = Volatile::new(&mut *((vaddr + 0xb8) as *mut u64));

            Self {
                version,
                capability,
                extended_capability,
                global_command,
                global_status,
                root_table_address,
                context_command,
                interrupt_remapping_table_addr,
                invalidate: InvalidationRegisters::new(vaddr),
            }
        };

        debug!("IOMMU registers:{:#x?}", iommu_regs);
        debug!("IOMMU capability:{:#x?}", iommu_regs.capability());
        debug!(
            "IOMMU extend capability:{:#x?}",
            iommu_regs.extended_capability()
        );

        Some(iommu_regs)
    }
}

pub(super) static IOMMU_REGS: Once<SpinLock<IommuRegisters>> = Once::new();

pub(super) fn init() -> Result<(), IommuError> {
    let iommu_regs = IommuRegisters::new().ok_or(IommuError::NoIommu)?;
    IOMMU_REGS.call_once(|| SpinLock::new(iommu_regs));
    Ok(())
}
