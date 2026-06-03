// SPDX-License-Identifier: MPL-2.0

//! The IOMMU support for RISC-V.
//!
//! Implements DMA remapping via a second-stage page table (Sv39x4) shared across
//! all devices, connected through a two-level Device Directory Table. Interrupt
//! remapping is not yet implemented.
//!
//! The public interface consists of [`has_dma_remapping`], [`map`], and [`unmap`],
//! which are called from the generic DMA layer in [`crate::mm::dma`]. Page table
//! configuration is provided by [`IommuPtConfig`], which implements
//! [`crate::mm::page_table::PageTableConfig`] for the RISC-V IOMMU's Sv39x4
//! second-stage translation format.
//!
//! See the parent module ([`crate::arch::riscv`]) for the initialization path.
//!
//! For more details, see the RISC-V IOMMU specification:
//! <https://docs.riscv.org/reference/iommu/index.html>.

// Set this module's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "iommu: "
    };
}

mod ddt;
mod dma_remapping;
mod fault;
mod queue;
mod registers;
mod second_stage;

pub(crate) use dma_remapping::{has_dma_remapping, map, unmap};
pub(crate) use second_stage::IommuPtConfig;

use crate::{io::IoMemAllocatorBuilder, mm::page_table::PageTableError};

/// An enumeration representing possible errors related to IOMMU.
#[derive(Debug)]
pub(crate) enum IommuError {
    /// No IOMMU is available.
    NoIommu,
    /// Error encountered during modification of the page table.
    ModificationError(PageTableError),
}

pub(crate) fn init(io_mem_builder: &IoMemAllocatorBuilder) -> Result<(), IommuError> {
    registers::init(io_mem_builder)?;
    dma_remapping::init();
    Ok(())
}

// The generic IRQ layer uses the return value to decide whether to allocate
// interrupt remapping table entries for MSI/MSI-X. Always `false` until
// interrupt remapping is implemented.
pub(crate) fn has_interrupt_remapping() -> bool {
    false
}
