// SPDX-License-Identifier: MPL-2.0

use super::{Interval, RssDelta, Vmar};
use crate::{prelude::*, vm::perms::VmPerms};

impl Vmar {
    pub fn handle_page_fault(&self, page_fault_info: &PageFaultInfo) -> Result<()> {
        self.handle_page_fault_with_report(page_fault_info)
            .map_err(PageFaultError::into_error)
    }

    /// Handles a page fault and reports the faulting address state on failure.
    pub(crate) fn handle_page_fault_with_report(
        &self,
        page_fault_info: &PageFaultInfo,
    ) -> core::result::Result<(), PageFaultError> {
        let inner = self.inner.read();

        let address = page_fault_info.address;
        if let Some(vm_mapping) = inner.vm_mappings.find_one(&address) {
            debug_assert!(vm_mapping.range().contains(&address));

            let mut rss_delta = RssDelta::new(self);
            return vm_mapping
                .handle_page_fault(&self.vm_space, page_fault_info, &mut rss_delta)
                .map_err(|error| PageFaultError::new(PageFaultAddressState::Mapped, error));
        }

        Err(PageFaultError::new(
            PageFaultAddressState::Unmapped,
            Error::with_message(
                Errno::EACCES,
                "no VM mappings contain the page fault address",
            ),
        ))
    }
}

/// An unhandled page fault with VMAR-level context.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PageFaultError {
    address_state: PageFaultAddressState,
    error: Error,
}

impl PageFaultError {
    fn new(address_state: PageFaultAddressState, error: Error) -> Self {
        Self {
            address_state,
            error,
        }
    }

    /// Returns whether the faulting address is covered by a VMAR mapping.
    pub(crate) fn address_state(&self) -> PageFaultAddressState {
        self.address_state
    }

    /// Returns the underlying page-fault handling error.
    pub(crate) fn error(&self) -> Error {
        self.error
    }

    fn into_error(self) -> Error {
        self.error
    }
}

/// Whether a faulting address is covered by a VMAR mapping.
#[derive(Clone, Copy, Debug)]
pub(crate) enum PageFaultAddressState {
    /// The faulting address is not covered by any VMAR mapping.
    Unmapped,
    /// The faulting address is covered by a VMAR mapping.
    Mapped,
}

/// Page fault information converted from [`CpuException`].
///
/// `TryFrom<CpuException>` should be implemented for this struct.
/// If [`CpuException`] is a page fault, `try_from` should return `Ok(PageFaultInfo)`,
/// or `Err(())` (no error information) otherwise.
///
/// [`CpuException`]: ostd::arch::cpu::context::CpuException
#[derive(Debug)]
pub struct PageFaultInfo {
    /// The virtual address where a page fault occurred.
    pub(in crate::vm::vmar) address: Vaddr,

    /// The [`VmPerms`] required by the memory operation that causes page fault.
    /// For example, a "store" operation may require `VmPerms::WRITE`.
    pub(in crate::vm::vmar) required_perms: VmPerms,

    /// Whether this page fault is forced (e.g., manually triggered by `ptrace`).
    /// A forced page fault may bypass some permission checks.
    is_forced: bool,
}

impl PageFaultInfo {
    /// Creates a new `PageFaultInfo`.
    pub fn new(address: Vaddr, required_perms: VmPerms) -> Self {
        Self {
            address,
            required_perms,
            is_forced: false,
        }
    }

    /// Returns whether this page fault is forced.
    pub(in crate::vm::vmar) fn is_forced(&self) -> bool {
        self.is_forced
    }

    /// Marks this page fault as forced.
    pub(super) fn force(mut self) -> Self {
        self.is_forced = true;
        self
    }
}
