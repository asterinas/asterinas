// SPDX-License-Identifier: MPL-2.0

mod table;

use core::fmt::Debug;

use log::{info, warn};
use spin::Once;
pub(super) use table::IntRemappingTable;

use crate::arch::iommu::registers::{ExtendedCapabilityFlags, IOMMU_REGS};

pub struct IrtEntryHandle {
    index: u16,
    table: &'static IntRemappingTable,
}

impl IrtEntryHandle {
    pub fn index(&self) -> u16 {
        self.index
    }

    pub fn enable(&self, vector: u32) {
        self.table
            .set_entry(self.index, table::IrtEntry::new_enabled(vector));
    }
}

impl Debug for IrtEntryHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IrtEntryHandle")
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}

pub fn has_interrupt_remapping() -> bool {
    REMAPPING_TABLE.get().is_some()
}

pub fn alloc_irt_entry() -> Option<IrtEntryHandle> {
    let page_table = REMAPPING_TABLE.get()?;
    page_table.alloc()
}

pub(super) fn init() {
    let mut iommu_regs = IOMMU_REGS.get().unwrap().lock();

    // Check if interrupt remapping is supported
    let extend_cap = iommu_regs.read_extended_capability();
    if !extend_cap.flags().contains(ExtendedCapabilityFlags::IR) {
        warn!("[IOMMU] Interrupt remapping not supported");
        return;
    }

    // Create interrupt remapping table
    REMAPPING_TABLE.call_once(IntRemappingTable::new);
    iommu_regs.enable_interrupt_remapping(REMAPPING_TABLE.get().unwrap());

    info!("[IOMMU] Interrupt remapping enabled");
}

static REMAPPING_TABLE: Once<IntRemappingTable> = Once::new();
