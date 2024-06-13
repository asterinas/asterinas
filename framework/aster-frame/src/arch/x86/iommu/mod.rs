// SPDX-License-Identifier: MPL-2.0

//! The IOMMU support.

mod context_table;
mod fault;
pub(crate) mod registers;
mod second_stage;

use log::info;
use registers::IOMMU_REGS;
pub use second_stage::DeviceMode;
use second_stage::{PageTableEntry, PagingConsts};
use spin::Once;

use crate::{
    arch::iommu::context_table::RootTable,
    bus::pci::PciDeviceLocation,
    mm::{dma::Daddr, page_table::PageTableError, Paddr, PageTable},
    sync::SpinLock,
};

/// An enumeration representing possible errors related to IOMMU.
#[derive(Debug)]
pub enum IommuError {
    /// No IOMMU is available.
    NoIommu,
    /// Error encountered during modification of the page table.
    ModificationError(PageTableError),
}

/// Mapping device address to physical address.
///
/// # Safety
///
/// Mapping an incorrect address may lead to a kernel data leak.
pub(crate) unsafe fn map(daddr: Daddr, paddr: Paddr) -> Result<(), IommuError> {
    let Some(table) = PAGE_TABLE.get() else {
        return Err(IommuError::NoIommu);
    };
    // The page table of all devices is the same. So we can use any device ID.
    table
        .lock()
        .map(PciDeviceLocation::zero(), daddr, paddr)
        .map_err(|err| match err {
            context_table::ContextTableError::InvalidDeviceId => unreachable!(),
            context_table::ContextTableError::ModificationError(err) => {
                IommuError::ModificationError(err)
            }
        })
}

pub(crate) fn unmap(daddr: Daddr) -> Result<(), IommuError> {
    let Some(table) = PAGE_TABLE.get() else {
        return Err(IommuError::NoIommu);
    };
    // The page table of all devices is the same. So we can use any device ID.
    table
        .lock()
        .unmap(PciDeviceLocation::zero(), daddr)
        .map_err(|err| match err {
            context_table::ContextTableError::InvalidDeviceId => unreachable!(),
            context_table::ContextTableError::ModificationError(err) => {
                IommuError::ModificationError(err)
            }
        })
}

pub(crate) fn init() -> Result<(), IommuError> {
    registers::init()?;

    // Create Root Table instance
    let mut root_table = RootTable::new();
    // For all PCI Device, use the same page table.
    let page_table = PageTable::<DeviceMode, PageTableEntry, PagingConsts>::empty();
    for table in PciDeviceLocation::all() {
        root_table.specify_device_page_table(table, unsafe { page_table.shallow_copy() })
    }
    PAGE_TABLE.call_once(|| SpinLock::new(root_table));

    // Enable DMA remapping
    let mut iommu_regs = IOMMU_REGS.get().unwrap().lock_irq_disabled();
    iommu_regs.enable_dma_remapping(PAGE_TABLE.get().unwrap());
    info!("[IOMMU] DMA remapping enabled");
    drop(iommu_regs);

    Ok(())
}

pub(crate) fn has_iommu() -> bool {
    PAGE_TABLE.get().is_some()
}

static PAGE_TABLE: Once<SpinLock<RootTable>> = Once::new();
