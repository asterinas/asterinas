// SPDX-License-Identifier: MPL-2.0

mod table;

use alloc::sync::Arc;
use core::{fmt::Debug, mem::size_of};

use log::{info, warn};
use spin::Once;
pub(super) use table::IntRemappingTable;
use table::IrtEntry;

use crate::{
    arch::iommu::registers::{ExtendedCapabilityFlags, IOMMU_REGS},
    prelude::Vaddr,
    sync::{LocalIrqDisabled, SpinLock},
};

pub struct IrtEntryHandle {
    index: u16,
    entry_ref: Option<&'static mut IrtEntry>,
}

impl IrtEntryHandle {
    pub fn index(&self) -> u16 {
        self.index
    }

    #[allow(unused)]
    pub fn irt_entry(&self) -> Option<&IrtEntry> {
        self.entry_ref.as_deref()
    }

    pub fn irt_entry_mut(&mut self) -> Option<&mut IrtEntry> {
        self.entry_ref.as_deref_mut()
    }

    /// Set entry reference to None.
    pub(self) fn set_none(&mut self) {
        self.entry_ref = None;
    }

    /// Creates a handle based on index and the interrupt remapping table base virtual address.
    ///
    /// # Safety
    ///
    /// User must ensure the target address is **always** valid and point to `IrtEntry`.
    pub(self) unsafe fn new(table_vaddr: Vaddr, index: u16) -> Self {
        Self {
            index,
            entry_ref: Some(
                &mut *((table_vaddr + index as usize * size_of::<IrtEntry>()) as *mut IrtEntry),
            ),
        }
    }
}

impl Debug for IrtEntryHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IrtEntryHandle")
            .field("index", &self.index)
            .field("entry_ref", &self.entry_ref)
            .finish()
    }
}

pub fn has_interrupt_remapping() -> bool {
    REMAPPING_TABLE.get().is_some()
}

pub fn alloc_irt_entry() -> Option<Arc<SpinLock<IrtEntryHandle, LocalIrqDisabled>>> {
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
