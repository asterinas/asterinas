// SPDX-License-Identifier: MPL-2.0

use super::perms::VmPerms;
use crate::prelude::*;

/// This trait is implemented by structs which can handle a user space page fault.
pub trait PageFaultHandler {
    /// Handle a page fault at the address `offset`.
    /// The `required_perms` indicates the [`VmPerms`] permission required by the memory operation.
    /// For example, read access reqiures [`VmPerms::READ`] while write access requires
    /// [`VmPerms::WRITE`].
    ///
    /// Returns `Ok` if the page fault is handled successfully, `Err` otherwise.
    fn handle_page_fault(&self, offset: Vaddr, required_perms: VmPerms) -> Result<()>;
}
