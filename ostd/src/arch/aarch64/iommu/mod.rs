// SPDX-License-Identifier: MPL-2.0

//! The IOMMU support (stub).

use crate::mm::{Daddr, Paddr};

/// IOMMU error type.
#[derive(Debug)]
pub(crate) enum IommuError {
    /// No IOMMU available.
    NoIommu,
}

/// IOMMU initialization stub.
pub(crate) fn init() -> Result<(), IommuError> {
    // TODO: Implement ARM SMMU support
    Err(IommuError::NoIommu)
}

pub(crate) fn has_dma_remapping() -> bool {
    false
}

pub(crate) fn has_interrupt_remapping() -> bool {
    false
}

/// Map a physical address to a DMA address.
///
/// # Safety
///
/// The caller must ensure the physical address is valid.
pub(crate) unsafe fn map(_daddr: Daddr, _paddr: Paddr) -> Result<(), IommuError> {
    Err(IommuError::NoIommu)
}

/// Unmap a DMA address.
pub(crate) fn unmap(_daddr: Daddr) -> Result<(), IommuError> {
    Err(IommuError::NoIommu)
}
