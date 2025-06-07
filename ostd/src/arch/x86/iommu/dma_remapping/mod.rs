// SPDX-License-Identifier: MPL-2.0

pub use context_table::RootTable;
use log::{info, warn};
use second_stage::IommuPtConfig;
use spin::Once;

use super::IommuError;
use crate::{
    arch::iommu::registers::{CapabilitySagaw, IOMMU_REGS},
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
    let mut locked_table = table.lock();
    // SAFETY: The safety is upheld by the caller.
    let res = unsafe { locked_table.map(PciDeviceLocation::zero(), daddr, paddr) };

    match res {
        Ok(()) => Ok(()),
        Err(context_table::ContextTableError::InvalidDeviceId) => unreachable!(),
        Err(context_table::ContextTableError::ModificationError(err)) => {
            Err(IommuError::ModificationError(err))
        }
    }
}

pub fn unmap(daddr: Daddr) -> Result<(), IommuError> {
    let Some(table) = PAGE_TABLE.get() else {
        return Err(IommuError::NoIommu);
    };

    // The page table of all devices is the same. So we can use any device ID.
    let mut locked_table = table.lock();
    let res = locked_table.unmap(PciDeviceLocation::zero(), daddr);

    match res {
        Ok(()) => Ok(()),
        Err(context_table::ContextTableError::InvalidDeviceId) => unreachable!(),
        Err(context_table::ContextTableError::ModificationError(err)) => {
            Err(IommuError::ModificationError(err))
        }
    }
}

pub fn init() {
    if !IOMMU_REGS
        .get()
        .unwrap()
        .lock()
        .read_capability()
        .supported_adjusted_guest_address_widths()
        .contains(CapabilitySagaw::AGAW_39BIT_3LP)
    {
        warn!("[IOMMU] 3-level page tables not supported, disabling DMA remapping");
        return;
    }

    // Create a Root Table instance.
    let mut root_table = RootTable::new();
    // For all PCI devices, use the same page table.
    //
    // TODO: The BIOS reserves some memory regions as DMA targets and lists them in the Reserved
    // Memory Region Reporting (RMRR) structures. These regions must be mapped for the hardware or
    // firmware to function properly. For more details, see Intel(R) Virtualization Technology for
    // Directed I/O (Revision 5.0), 3.16 Handling Requests to Reserved System Memory.
    let page_table = PageTable::<IommuPtConfig>::empty();
    for table in PciDeviceLocation::all() {
        root_table.specify_device_page_table(table, unsafe { page_table.shallow_copy() })
    }
    PAGE_TABLE.call_once(|| SpinLock::new(root_table));

    // Enable DMA remapping.
    let mut iommu_regs = IOMMU_REGS.get().unwrap().lock();
    iommu_regs.enable_dma_remapping(PAGE_TABLE.get().unwrap());
    info!("[IOMMU] DMA remapping enabled");
}

// TODO: Currently `map()` or `unmap()` could be called in both task and interrupt
// contexts (e.g., within the virtio-blk module), potentially leading to deadlocks.
// Once this issue is resolved, `LocalIrqDisabled` is no longer needed.
static PAGE_TABLE: Once<SpinLock<RootTable, LocalIrqDisabled>> = Once::new();
