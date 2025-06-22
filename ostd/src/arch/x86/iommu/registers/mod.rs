// SPDX-License-Identifier: MPL-2.0

//! Registers and their definition used by IOMMU.

mod capability;
mod command;
mod extended_cap;
mod invalidation;
mod status;

use core::ptr::NonNull;

use bit_field::BitField;
pub use capability::{Capability, CapabilitySagaw};
use command::GlobalCommand;
use extended_cap::ExtendedCapability;
pub use extended_cap::ExtendedCapabilityFlags;
use invalidation::InvalidationRegisters;
use log::debug;
use spin::Once;
use status::GlobalStatus;
use volatile::{
    access::{ReadOnly, ReadWrite, WriteOnly},
    VolatileRef,
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
        kernel::acpi::dmar::{Dmar, Remapping},
    },
    io::IoMemAllocatorBuilder,
    mm::{paddr_to_vaddr, PAGE_SIZE},
    sync::{LocalIrqDisabled, SpinLock},
};

#[derive(Debug, Clone, Copy)]
pub struct IommuVersion {
    major: u8,
    minor: u8,
}

impl IommuVersion {
    /// Major version number
    #[expect(dead_code)]
    pub fn major(&self) -> u8 {
        self.major
    }

    /// Minor version number
    #[expect(dead_code)]
    pub fn minor(&self) -> u8 {
        self.minor
    }
}

/// Important registers used by IOMMU.
#[derive(Debug)]
pub struct IommuRegisters {
    version: VolatileRef<'static, u32, ReadOnly>,
    capability: VolatileRef<'static, u64, ReadOnly>,
    extended_capability: VolatileRef<'static, u64, ReadOnly>,
    global_command: VolatileRef<'static, u32, WriteOnly>,
    global_status: VolatileRef<'static, u32, ReadOnly>,
    root_table_address: VolatileRef<'static, u64, ReadWrite>,
    context_command: VolatileRef<'static, u64, ReadWrite>,

    interrupt_remapping_table_addr: VolatileRef<'static, u64, ReadWrite>,

    invalidate: InvalidationRegisters,
}

impl IommuRegisters {
    /// Reads the version of IOMMU
    #[expect(dead_code)]
    pub fn read_version(&self) -> IommuVersion {
        let version = self.version.as_ptr().read();
        IommuVersion {
            major: version.get_bits(4..8) as u8,
            minor: version.get_bits(0..4) as u8,
        }
    }

    /// Reads the capability of IOMMU
    pub fn read_capability(&self) -> Capability {
        Capability::new(self.capability.as_ptr().read())
    }

    /// Reads the extended Capability of IOMMU
    pub fn read_extended_capability(&self) -> ExtendedCapability {
        ExtendedCapability::new(self.extended_capability.as_ptr().read())
    }

    /// Reads the global Status of IOMMU
    pub fn read_global_status(&self) -> GlobalStatus {
        GlobalStatus::from_bits_truncate(self.global_status.as_ptr().read())
    }

    /// Enables DMA remapping with static RootTable
    pub(super) fn enable_dma_remapping(
        &mut self,
        root_table: &'static SpinLock<RootTable, LocalIrqDisabled>,
    ) {
        // Set root table address
        self.root_table_address
            .as_mut_ptr()
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
        self.interrupt_remapping_table_addr
            .as_mut_ptr()
            .write(table.encode());
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
            self.invalidate
                .queue_tail
                .as_mut_ptr()
                .write((tail << 4) as u64);
            while (self.invalidate.queue_head.as_ptr().read() >> 4) + 1 == tail as u64 {}

            // We need to set the interrupt flag so that the `Invalidation Completion Status Register` can report the completion status.
            queue.append_descriptor(InvalidationWait::with_interrupt_flag().0);
            self.invalidate
                .queue_tail
                .as_mut_ptr()
                .write((queue.tail() << 4) as u64);

            // Wait for completion
            while self.invalidate.completion_status.as_ptr().read() == 0 {}
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
        self.invalidate.queue_tail.as_mut_ptr().write(0);

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

        self.invalidate.queue_addr.as_mut_ptr().write(write_value);

        // Enable Queued invalidation
        self.write_global_command(GlobalCommand::QIE, true);
        while !self.read_global_status().contains(GlobalStatus::QIES) {}
    }

    fn global_invalidation(&mut self) {
        // Set ICC(63) to 1 to requests invalidation and CIRG(62:61) to 01 to indicate global invalidation request.
        self.context_command
            .as_mut_ptr()
            .write(0xA000_0000_0000_0000);

        // Wait for invalidation complete (ICC set to 0).
        let mut value = 0x8000_0000_0000_0000;
        while (value & 0x8000_0000_0000_0000) != 0 {
            value = self.context_command.as_ptr().read();
        }

        // Set IVT(63) to 1 to requests IOTLB invalidation and IIRG(61:60) to 01 to indicate global invalidation request.
        self.invalidate
            .iotlb_invalidate
            .as_mut_ptr()
            .write(0x9000_0000_0000_0000);
    }

    /// Writes value to the global command register. This function will not wait until the command
    /// is serviced. User need to check the global status register.
    fn write_global_command(&mut self, command: GlobalCommand, enable: bool) {
        const ONE_SHOT_STATUS_MASK: u32 = 0x96FF_FFFF;
        let status = self.global_status.as_ptr().read() & ONE_SHOT_STATUS_MASK;
        if enable {
            self.global_command
                .as_mut_ptr()
                .write(status | command.bits());
        } else {
            self.global_command
                .as_mut_ptr()
                .write(status & !command.bits());
        }
    }

    /// Creates an instance from base address
    fn new(io_mem_builder: &IoMemAllocatorBuilder) -> Option<Self> {
        let dmar = Dmar::new()?;
        debug!("DMAR: {:#x?}", dmar);

        let base_address = dmar
            .remapping_iter()
            // TODO: Add support for multiple DMA remapping hardware unit definitions (DRHDs). Note
            // that we use `rev()` here to select the last one, since DRHDs that control specific
            // devices tend to be reported first.
            //
            // For example, Intel(R) Virtualization Technology for Directed I/O (Revision 5.0), 8.4
            // DMA Remapping Hardware Unit Definition Structure says "If a DRHD structure with
            // INCLUDE_PCI_ALL flag Set is reported for a Segment, it must be enumerated by BIOS
            // after all other DRHD structures for the same Segment".
            .rev()
            .find_map(|remapping| match remapping {
                Remapping::Drhd(drhd) => Some(drhd.register_base_addr()),
                _ => None,
            })
            .expect("no DRHD structure found in the DMAR table");
        assert_ne!(base_address, 0, "IOMMU address should not be zero");
        debug!("IOMMU base address: {:#x?}", base_address);

        io_mem_builder.remove(base_address as usize..(base_address as usize + PAGE_SIZE));
        let base = NonNull::new(paddr_to_vaddr(base_address as usize) as *mut u8).unwrap();

        // SAFETY:
        // - We trust the ACPI tables (as well as the DRHD in them), from which the base address is
        //   obtained, so it is a valid IOMMU base address.
        // - `io_mem_builder.remove()` guarantees that we have exclusive ownership of all the IOMMU
        //   registers.
        let iommu_regs = unsafe {
            fault::init(base);

            Self {
                version: VolatileRef::new_read_only(base.cast::<u32>()),
                capability: VolatileRef::new_read_only(base.add(0x08).cast::<u64>()),
                extended_capability: VolatileRef::new_read_only(base.add(0x10).cast::<u64>()),
                global_command: VolatileRef::new_restricted(
                    WriteOnly,
                    base.add(0x18).cast::<u32>(),
                ),
                global_status: VolatileRef::new_read_only(base.add(0x1C).cast::<u32>()),
                root_table_address: VolatileRef::new(base.add(0x20).cast::<u64>()),
                context_command: VolatileRef::new(base.add(0x28).cast::<u64>()),

                interrupt_remapping_table_addr: VolatileRef::new(base.add(0xb8).cast::<u64>()),

                invalidate: InvalidationRegisters::new(base),
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

pub(super) fn init(io_mem_builder: &IoMemAllocatorBuilder) -> Result<(), IommuError> {
    let iommu_regs = IommuRegisters::new(io_mem_builder).ok_or(IommuError::NoIommu)?;
    IOMMU_REGS.call_once(|| SpinLock::new(iommu_regs));
    Ok(())
}
