// SPDX-License-Identifier: MPL-2.0

//! The IOMMU support.

mod context_table;
mod fault;
mod remapping;
mod second_stage;

use log::info;
pub use second_stage::DeviceMode;
use second_stage::{PageTableEntry, PagingConsts};
use spin::Once;

use crate::{
    arch::iommu::context_table::RootTable,
    bus::pci::PciDeviceLocation,
    mm::{dma::Daddr, page_table::PageTableError, Paddr, PageTable},
    sync::Mutex,
};

/// An enumeration representing possible errors related to IOMMU.
#[derive(Debug)]
pub enum IommuError {
    /// No IOMMU is available.
    NoIommu,
    /// Error encountered during modification of the page table.
    ModificationError(PageTableError),
}

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
    let mut root_table = RootTable::new();
    // For all PCI Device, use the same page table.
    let page_table = PageTable::<DeviceMode, PageTableEntry, PagingConsts>::empty();
    for table in PciDeviceLocation::all() {
        root_table.specify_device_page_table(table, unsafe { page_table.shallow_copy() })
    }
    remapping::init(&root_table)?;
    PAGE_TABLE.call_once(|| Mutex::new(root_table));
    info!("IOMMU enabled");
    Ok(())
}

pub(crate) fn has_iommu() -> bool {
    PAGE_TABLE.get().is_some()
}

static PAGE_TABLE: Once<Mutex<RootTable>> = Once::new();
