// SPDX-License-Identifier: MPL-2.0

use crate::mm::page_table::PageTableError;

/// The error type which is returned from the APIs of this crate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// Invalid arguments provided.
    InvalidArgs,
    /// Insufficient memory available.
    NoMemory,
    /// Page fault occurred.
    PageFault,
    /// Access to a resource is denied.
    AccessDenied,
    /// Input/output error.
    IoError,
    /// Insufficient system resources.
    NotEnoughResources,
    /// Arithmetic Overflow occurred.
    Overflow,
    /// Memory mapping already exists for the given virtual address.
    MapAlreadyMappedVaddr,
    /// Error when allocating kernel virtual memory.
    KVirtAreaAllocError,
}

impl From<PageTableError> for Error {
    fn from(_err: PageTableError) -> Error {
        Error::AccessDenied
    }
}
