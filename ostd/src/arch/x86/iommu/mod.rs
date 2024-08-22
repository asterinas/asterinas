// SPDX-License-Identifier: MPL-2.0

//! The IOMMU support.

mod dma_remapping;
mod fault;
mod interrupt_remapping;
mod invalidate;
mod registers;

pub(crate) use dma_remapping::{has_dma_remapping, map, unmap};
pub(crate) use interrupt_remapping::{alloc_irt_entry, has_interrupt_remapping, IrtEntryHandle};

use crate::mm::page_table::PageTableError;

/// An enumeration representing possible errors related to IOMMU.
#[derive(Debug)]
pub enum IommuError {
    /// No IOMMU is available.
    NoIommu,
    /// Error encountered during modification of the page table.
    ModificationError(PageTableError),
}

pub(crate) fn init() -> Result<(), IommuError> {
    registers::init()?;
    invalidate::init();
    dma_remapping::init();
    interrupt_remapping::init();
    Ok(())
}
