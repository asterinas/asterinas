// SPDX-License-Identifier: MPL-2.0

//! Registers and their definition used by IOMMU.

mod capability;
mod command;
mod extended_cap;
mod invalidation;
mod status;

use bit_field::BitField;
pub use capability::Capability;
use command::GlobalCommand;
use extended_cap::ExtendedCapability;
pub use extended_cap::ExtendedCapabilityFlags;
use invalidation::InvalidationRegisters;
use log::debug;
use spin::Once;
use status::GlobalStatus;
use volatile::{
    access::{ReadOnly, ReadWrite, WriteOnly},
    Volatile,
};

use super::{
    dma_remapping::RootTable, interrupt_remapping::IntRemappingTable, invalidate::queue::Queue,
    IommuError,
};
use crate::{
    arch::{
        iommu::{
            fault,
            invalidate::{
                descriptor::{InterruptEntryCache, InvalidationWait},
                QUEUE,
            },
        },
        x86::kernel::acpi::dmar::{Dmar, Remapping},
    },
    mm::paddr_to_vaddr,
    sync::{LocalIrqDisabled, SpinLock},
};

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct IommuVersion {
    major: u8,
    minor: u8,
}

impl IommuVersion {
    /// Major version number
    #[allow(dead_code)]
    pub fn major(&self) -> u8 {
        self.major
    }

    /// Minor version number
    #[allow(dead_code)]
    pub fn minor(&self) -> u8 {
        self.minor
    }
}

/// Important registers used by IOMMU.
#[derive(Debug)]
pub struct IommuRegisters {
    #[allow(dead_code)]
    version: Volatile<&'static u32, ReadOnly>,
    capability: Volatile<&'static u64, ReadOnly>,
    extended_capability: Volatile<&'static u64, ReadOnly>,
    global_command: Volatile<&'static mut u32, WriteOnly>,
    global_status: Volatile<&'static u32, ReadOnly>,
    root_table_address: Volatile<&'static mut u64, ReadWrite>,
    #[allow(dead_code)]
    context_command: Volatile<&'static mut u64, ReadWrite>,

    interrupt_remapping_table_addr: Volatile<&'static mut u64, ReadWrite>,

    invalidate: InvalidationRegisters,
}

impl IommuRegisters {
    /// Reads the version of IOMMU
    #[allow(dead_code)]
    pub fn read_version(&self) -> IommuVersion {
        let version = self.version.read();
        IommuVersion {
            major: version.get_bits(4..8) as u8,
            minor: version.get_bits(0..4) as u8,
        }
    }

    /// Reads the capability of IOMMU
    pub fn read_capability(&self) -> Capability {
        Capability::new(self.capability.read())
    }

    /// Reads the extended Capability of IOMMU
    pub fn read_extended_capability(&self) -> ExtendedCapability {
        ExtendedCapability::new(self.extended_capability.read())
    }

    /// Reads the global Status of IOMMU
    pub fn read_global_status(&self) -> GlobalStatus {
        GlobalStatus::from_bits_truncate(self.global_status.read())
    }

    /// Enables DMA remapping with static RootTable
    pub(super) fn enable_dma_remapping(
        &mut self,
        root_table: &'static SpinLock<RootTable, LocalIrqDisabled>,
    ) {
        // Set root table address
        self.root_table_address
            .write(root_table.lock().root_paddr() as u64);
        self.write_global_command(GlobalCommand::SRTP, true);
        while !self.read_global_status().contains(GlobalStatus::RTPS) {}

        // Enable DMA remapping
        self.write_global_command(GlobalCommand::TE, true);
        while !self.read_global_status().contains(GlobalStatus::TES) {}
    }

    /// Enables Interrupt Remapping with IntRemappingTable
    pub(super) fn enable_interrupt_remapping(&mut self, table: &'static IntRemappingTable) {
        assert!(self
            .read_extended_capability()
            .flags()
            .contains(ExtendedCapabilityFlags::IR));
        // Set interrupt remapping table address
        self.interrupt_remapping_table_addr.write(table.encode());
        self.write_global_command(GlobalCommand::SIRTP, true);
        while !self.read_global_status().contains(GlobalStatus::IRTPS) {}

        // Enable Interrupt Remapping
        self.write_global_command(GlobalCommand::IRE, true);
        while !self.read_global_status().contains(GlobalStatus::IRES) {}

        // Invalidate interrupt cache
        if self.read_global_status().contains(GlobalStatus::QIES) {
            let mut queue = QUEUE.get().unwrap().lock();

            // Construct global invalidation of interrupt cache and invalidation wait.
            queue.append_descriptor(InterruptEntryCache::global_invalidation().0);
            let tail = queue.tail();
            self.invalidate.queue_tail.write((tail << 4) as u64);
            while (self.invalidate.queue_head.read() >> 4) + 1 == tail as u64 {}

            // We need to set the interrupt flag so that the `Invalidation Completion Status Register` can report the completion status.
            queue.append_descriptor(InvalidationWait::with_interrupt_flag().0);
            self.invalidate.queue_tail.write((queue.tail() << 4) as u64);

            // Wait for completion
            while self.invalidate.completion_status.read() == 0 {}
        } else {
            self.global_invalidation()
        }

        // Disable Compatibility format interrupts
        if self.read_global_status().contains(GlobalStatus::CFIS) {
            self.write_global_command(GlobalCommand::CFI, false);
            while self.read_global_status().contains(GlobalStatus::CFIS) {}
        }
    }

    pub(super) fn enable_queued_invalidation(&mut self, queue: &Queue) {
        assert!(self
            .read_extended_capability()
            .flags()
            .contains(ExtendedCapabilityFlags::QI));
        self.invalidate.queue_tail.write(0);

        let mut write_value = queue.base_paddr() as u64;
        // By default, we set descriptor width to 128-bit(0)
        let descriptor_width = 0b0;
        write_value |= descriptor_width << 11;

        let write_queue_size = {
            let mut queue_size = queue.size();
            assert!(queue_size.is_power_of_two());
            let mut write_queue_size = 0;

            if descriptor_width == 0 {
                // 2^(write_queue_size + 8) = number of entries = queue_size
                assert!(queue_size >= (1 << 8));
                queue_size >>= 8;
            } else {
                // 2^(write_queue_size + 7) = number of entries = queue_size
                assert!(queue_size >= (1 << 7));
                queue_size >>= 7;
            };

            while queue_size & 0b1 == 0 {
                queue_size >>= 1;
                write_queue_size += 1;
            }
            write_queue_size
        };

        write_value |= write_queue_size;

        self.invalidate.queue_addr.write(write_value);

        // Enable Queued invalidation
        self.write_global_command(GlobalCommand::QIE, true);
        while !self.read_global_status().contains(GlobalStatus::QIES) {}
    }

    fn global_invalidation(&mut self) {
        // Set ICC(63) to 1 to requests invalidation and CIRG(62:61) to 01 to indicate global invalidation request.
        self.context_command.write(0xA000_0000_0000_0000);

        // Wait for invalidation complete (ICC set to 0).
        let mut value = 0x8000_0000_0000_0000;
        while (value & 0x8000_0000_0000_0000) != 0 {
            value = self.context_command.read();
        }

        // Set IVT(63) to 1 to requests IOTLB invalidation and IIRG(61:60) to 01 to indicate global invalidation request.
        self.invalidate
            ._iotlb_invalidate
            .write(0x9000_0000_0000_0000);
    }

    /// Writes value to the global command register. This function will not wait until the command
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

    /// Creates an instance from base address
    fn new() -> Option<Self> {
        let dmar = Dmar::new()?;

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
                invalidate: InvalidationRegisters::new(vaddr),
                interrupt_remapping_table_addr,
            }
        };

        debug!("IOMMU registers:{:#x?}", iommu_regs);
        debug!("IOMMU capability:{:#x?}", iommu_regs.read_capability());
        debug!(
            "IOMMU extend capability:{:#x?}",
            iommu_regs.read_extended_capability()
        );

        Some(iommu_regs)
    }
}

pub(super) static IOMMU_REGS: Once<SpinLock<IommuRegisters, LocalIrqDisabled>> = Once::new();

pub(super) fn init() -> Result<(), IommuError> {
    let iommu_regs = IommuRegisters::new().ok_or(IommuError::NoIommu)?;
    IOMMU_REGS.call_once(|| SpinLock::new(iommu_regs));
    Ok(())
}
