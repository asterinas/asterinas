// SPDX-License-Identifier: MPL-2.0

//! Registers and their definition used by IOMMU.

mod capability;
mod command;
mod extended_cap;
mod status;

use bit_field::BitField;
pub use capability::Capability;
use command::GlobalCommand;
use extended_cap::ExtendedCapability;
use log::debug;
use spin::Once;
use status::GlobalStatus;
use volatile::{
    access::{ReadOnly, ReadWrite, WriteOnly},
    Volatile,
};

use super::{dma_remapping::RootTable, IommuError};
use crate::{
    arch::{
        iommu::fault,
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
}

impl IommuRegisters {
    /// Version of IOMMU
    #[allow(dead_code)]
    pub fn version(&self) -> IommuVersion {
        let version = self.version.read();
        IommuVersion {
            major: version.get_bits(4..8) as u8,
            minor: version.get_bits(0..4) as u8,
        }
    }

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
    pub(super) fn enable_dma_remapping(
        &mut self,
        root_table: &'static SpinLock<RootTable, LocalIrqDisabled>,
    ) {
        // Set root table address
        self.root_table_address
            .write(root_table.lock_with(|t| t.root_paddr()) as u64);
        self.write_global_command(GlobalCommand::SRTP, true);
        while !self.global_status().contains(GlobalStatus::RTPS) {}

        // Enable DMA remapping
        self.write_global_command(GlobalCommand::TE, true);
        while !self.global_status().contains(GlobalStatus::TES) {}
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
            Self {
                version,
                capability,
                extended_capability,
                global_command,
                global_status,
                root_table_address,
                context_command,
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
