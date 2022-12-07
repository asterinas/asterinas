use jinux_frame::vm::Vaddr;
use jinux_frame::Result;

/// This trait is implemented by structs which can handle a user space page fault.
/// In current implementation, they are vmars and vmos.
pub trait PageFaultHandler {
    /// Handle a page fault at a specific addr. if write is true, means the page fault is caused by a write access,
    /// otherwise, the page fault is caused by a read access.
    /// If the page fault can be handled successfully, this function will return Ok(()).
    /// Otherwise, this function will return Err.
    fn handle_page_fault(&self, page_fault_addr: Vaddr, write: bool) -> Result<()>;
}
