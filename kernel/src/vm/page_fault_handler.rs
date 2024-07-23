// SPDX-License-Identifier: MPL-2.0

use crate::{prelude::*, thread::exception::PageFaultInfo};

/// This trait is implemented by structs which can handle a user space page fault.
pub trait PageFaultHandler {
    /// Handle a page fault, whose information is provided in `page_fault_info`.
    ///
    /// Returns `Ok` if the page fault is handled successfully, `Err` otherwise.
    fn handle_page_fault(&self, page_fault_info: &PageFaultInfo) -> Result<()>;
}
