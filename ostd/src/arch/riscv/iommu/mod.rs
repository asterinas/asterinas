// SPDX-License-Identifier: MPL-2.0

//! The IOMMU support.

use crate::mm::{dma::Daddr, Paddr};

/// An enumeration representing possible errors related to IOMMU.
#[derive(Debug)]
pub enum IommuError {
    /// No IOMMU is available.
    NoIommu,
}

///
/// # Safety
///
/// Mapping an incorrect address may lead to a kernel data leak.
pub(crate) unsafe fn map(_daddr: Daddr, _paddr: Paddr) -> Result<(), IommuError> {
    Err(IommuError::NoIommu)
}

pub(crate) fn unmap(_daddr: Daddr) -> Result<(), IommuError> {
    Err(IommuError::NoIommu)
}

pub(crate) fn init() -> Result<(), IommuError> {
    // TODO: We will support IOMMU on RISC-V
    Err(IommuError::NoIommu)
}

pub(crate) fn has_dma_remapping() -> bool {
    false
}

pub(crate) fn has_interrupt_remapping() -> bool {
    false
}
