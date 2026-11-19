//! Guest physical memory space.
// ****************************************************************************
// *                                                                          *
// *  Why does this file exist?                                               *
// *                                                                          *
// ****************************************************************************
//
//   Currently, the kernel's page tables only support VPNs with a uniform bit 
//   width, whereas G-stage page tables require the root page table's VPN field 
//   to be expanded to 11 bits; for instance, in Sv48*4 mode, the VPN structure 
//   is [11, 9, 9, 9], rather than the uniform 9-bit segments found in the 
//   standard Sv48 page tables currently used by the kernel.
//
//   G-stage docs: https://docs.riscv.org/reference/isa/v20260120/priv/hypervisor.html#two-stage-translation
//   issue: https://github.com/asterinas/asterinas/issues/4016
//
// ----------------------------------------------------------------------------
//
//   NOTE:
//       The code in this file currently does not comply with coding standards.
//
//
//       This file provides only a minimal functional implementation.
//
//       Once the kernel supports this feature, this file can be rewritten after 
//       replacement and testing are complete.
//
//   NOTE:
//       Files should be checked if this file is going to be deleted:
//
//       1.ostd/src/arch/riscv/vm/gstage.rs
//       2.ostd/src/vm/mod.rs
//       3.ostd/src/vm/gpm_space.rs
//       4.ostd/src/vm/temp_riscv_gpm.rs
//       5.kernel/src/kvm/vm_memory.rs(func map_guest_memory() & unmap_guest_memory())
//
// ****************************************************************************


use crate::{
    arch::vm::gstage::*, mm::{PageProperty}, 
    prelude::*, 
};

use crate::sync::SpinLock;

/// Manages the guest physical memory space of a VM.
pub struct GuestPhysMemSpace {
    pt: SpinLock<GStagePageTable>,
}

impl GuestPhysMemSpace {
    /// Creates a new guest physical memory space.
    pub fn new() -> Result<Self> {
        Ok(Self {
            pt: SpinLock::new(GStagePageTable::new()),
        })
    }

    /// Map
    pub fn map(&self, gpa: usize, hpa: usize, size: usize, prop: PageProperty) -> Result<()> {
        self.pt.lock().map(gpa as u64, hpa as u64, size, prop.flags.bits() as u64)?;
        Ok(())
    }

    /// Unmap
    pub fn unmap(&self, addr_start: usize, addr_end: usize) -> Result<usize> {
        self.pt.lock().unmap(addr_start as u64, addr_end - addr_start)
    }

    /// Returns the root pointer value for this guest memory space.
    pub fn root_paddr(&self) -> u64 {
        self.pt.lock().get_root()
    }
}

impl Drop for GuestPhysMemSpace {
    fn drop(&mut self) {
        debug!("hypervisor: release guest memory space.");
    }
}
