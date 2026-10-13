//! Guest physical memory space.

use crate::{
    arch::vm::gstage::*, mm::{PageProperty}, 
    prelude::*, 
};

use crate::sync::SpinLock;

/// Manages the guest physical memory space of a VM.
///
/// This type owns the page table that maps guest physical addresses to
/// host physical frames. One `GuestPhysMemSpace` can be reused by multiple
/// vCPUs in the same VM by passing a reference to
/// [`super::GuestMode::execute`].
pub struct GuestPhysMemSpace {
    pt: SpinLock<GStagePageTable>,
}

impl GuestPhysMemSpace {
    /// Creates a new guest physical memory space.
    ///
    /// # Errors
    /// Returns an error if the CPU does not support second-stage address
    /// translation.
    pub fn new() -> Result<Self> {
        Ok(Self {
            pt: SpinLock::new(GStagePageTable::new()),
        })
    }

    /// 1
    pub fn map(&self, gpa: usize, hpa: usize, size: usize, prop: PageProperty) -> Result<()> {
        self.pt.lock().map(gpa as u64, hpa as u64, size, prop.flags.bits() as u64)?;
        Ok(())
    }

    /// 1
    pub fn unmap(&self, addr_start: usize, addr_end: usize) -> Result<usize> {
        self.pt.lock().unmap(addr_start as u64, addr_end - addr_start)
    }

    /// Returns the EPT pointer value for this guest memory space.
    ///
    /// The value is used by [`super::GuestMode`] so VM entry can use this EPT
    /// as the guest physical address space.
    pub fn root_paddr(&self) -> u64 {
        self.pt.lock().get_root()
    }
}

impl Drop for GuestPhysMemSpace {
    fn drop(&mut self) {
        debug!("hypervisor: release guest memory space.");
    }
}
