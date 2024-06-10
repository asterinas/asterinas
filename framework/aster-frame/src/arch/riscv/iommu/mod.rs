// SPDX-License-Identifier: MPL-2.0

use crate::vm::{dma::Daddr, page_table::PageTableError, Paddr};

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
    Err(IommuError::NoIommu)
}

pub(crate) fn unmap(daddr: Daddr) -> Result<(), IommuError> {
    Err(IommuError::NoIommu)
}

pub(crate) fn init() -> Result<(), IommuError> {
    Ok(())
}

pub(crate) fn has_iommu() -> bool {
    false
}
