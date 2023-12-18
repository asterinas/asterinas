use crate::prelude::*;

/// This trait is implemented by structs which can handle a user space page fault.
pub trait PageFaultHandler {
    /// Handle a page fault at a specific addr. if not_present is true, the page fault is caused by page not present.
    /// Otherwise, it's caused by page protection error.
    /// if write is true, the page fault is caused by a write access,
    /// otherwise, the page fault is caused by a read access.
    /// If the page fault can be handled successfully, this function will return Ok(()).
    /// Otherwise, this function will return Err.
    fn handle_page_fault(&self, offset: Vaddr, not_present: bool, write: bool) -> Result<()>;
}
