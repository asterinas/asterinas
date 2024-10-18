// SPDX-License-Identifier: MPL-2.0

pub use context_table::RootTable;
use log::info;
use second_stage::{DeviceMode, PageTableEntry, PagingConsts};
use spin::Once;

use super::IommuError;
use crate::{
    arch::iommu::registers::IOMMU_REGS,
    bus::pci::PciDeviceLocation,
    mm::{Daddr, PageTable},
    prelude::Paddr,
    sync::{LocalIrqDisabled, SpinLock},
};

mod context_table;
mod second_stage;

pub fn has_dma_remapping() -> bool {
    PAGE_TABLE.get().is_some()
}

/// Mapping device address to physical address.
///
/// # Safety
///
/// Mapping an incorrect address may lead to a kernel data leak.
pub unsafe fn map(daddr: Daddr, paddr: Paddr) -> Result<(), IommuError> {
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

pub fn unmap(daddr: Daddr) -> Result<(), IommuError> {
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

pub fn init() {
    // Create Root Table instance
    let mut root_table = RootTable::new();
    // For all PCI Device, use the same page table.
    let page_table = PageTable::<DeviceMode, PageTableEntry, PagingConsts>::empty();
    for table in PciDeviceLocation::all() {
        root_table.specify_device_page_table(table, unsafe { page_table.shallow_copy() })
    }
    PAGE_TABLE.call_once(|| SpinLock::new(root_table));

    // Enable DMA remapping
    let mut iommu_regs = IOMMU_REGS.get().unwrap().lock();
    iommu_regs.enable_dma_remapping(PAGE_TABLE.get().unwrap());
    info!("[IOMMU] DMA remapping enabled");
}

// TODO: Currently `map()` or `unmap()` could be called in both task and interrupt
// contexts (e.g., within the virtio-blk module), potentially leading to deadlocks.
// Once this issue is resolved, `LocalIrqDisabled` is no longer needed.
static PAGE_TABLE: Once<SpinLock<RootTable, LocalIrqDisabled>> = Once::new();
