// SPDX-License-Identifier: MPL-2.0

pub use context_table::RootTable;
use log::{info, warn};
pub use second_stage::IommuPtConfig;
use spin::Once;

use super::IommuError;
use crate::{
    arch::iommu::registers::{CapabilitySagaw, IOMMU_REGS},
    mm::{Daddr, PageTable},
    prelude::Paddr,
    sync::{LocalIrqDisabled, SpinLock},
};

mod context_table;
mod second_stage;

pub fn has_dma_remapping() -> bool {
    PAGE_TABLE.get().is_some()
}

/// PCI device Location
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PciDeviceLocation {
    /// Bus number
    pub bus: u8,
    /// Device number with max 31
    pub device: u8,
    /// Device number with max 7
    pub function: u8,
}

impl PciDeviceLocation {
    // TODO: Find a proper way to obtain the bus range. For example, if the PCI bus is identified
    // from a device tree, this information can be obtained from the `bus-range` field (e.g.,
    // `bus-range = <0x00 0x7f>`).
    const MIN_BUS: u8 = 0;
    const MAX_BUS: u8 = 255;

    const MIN_DEVICE: u8 = 0;
    const MAX_DEVICE: u8 = 31;

    const MIN_FUNCTION: u8 = 0;
    const MAX_FUNCTION: u8 = 7;

    /// Returns an iterator that enumerates all possible PCI device locations.
    fn all() -> impl Iterator<Item = PciDeviceLocation> {
        let all_bus = Self::MIN_BUS..=Self::MAX_BUS;
        let all_dev = Self::MIN_DEVICE..=Self::MAX_DEVICE;
        let all_func = Self::MIN_FUNCTION..=Self::MAX_FUNCTION;

        all_bus
            .flat_map(move |bus| all_dev.clone().map(move |dev| (bus, dev)))
            .flat_map(move |(bus, dev)| all_func.clone().map(move |func| (bus, dev, func)))
            .map(|(bus, dev, func)| PciDeviceLocation {
                bus,
                device: dev,
                function: func,
            })
    }

    /// Returns the zero PCI device location.
    fn zero() -> Self {
        Self {
            bus: 0,
            device: 0,
            function: 0,
        }
    }
}

/// Maps a device address to a physical address.
///
/// The physical address should point to a page containing untyped, non-sensitive data that can be
/// accessed by the device.
///
/// # Safety
///
/// While the physical address is mapped as the device address (i.e. from calling this method to
/// calling [`unmap`]), it must point to an untyped memory page. Otherwise, the device may corrupt
/// kernel data, which could lead to memory safety issues.
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

/// Unmaps a device address.
///
/// This method will fail if the device address is not mapped (by [`map`]) before.
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
