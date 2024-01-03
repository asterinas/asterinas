// SPDX-License-Identifier: MPL-2.0

mod context_table;
mod fault;
mod remapping;
mod second_stage;

use log::info;
use spin::Once;

use crate::{
    arch::iommu::{context_table::RootTable, second_stage::PageTableEntry},
    bus::pci::PciDeviceLocation,
    sync::Mutex,
    vm::{
        dma::Daddr,
        page_table::{DeviceMode, PageTableConfig, PageTableError},
        Paddr, PageTable,
    },
};

#[derive(Debug)]
pub enum IommuError {
    NoIommu,
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
    let page_table: PageTable<PageTableEntry, DeviceMode> =
        PageTable::<PageTableEntry, DeviceMode>::new(PageTableConfig {
            address_width: crate::vm::page_table::AddressWidth::Level3,
        });
    for table in PciDeviceLocation::all() {
        root_table.specify_device_page_table(table, page_table.clone())
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
