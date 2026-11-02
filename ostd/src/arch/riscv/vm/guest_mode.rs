use crate::prelude::*;
use crate::vm::{GuestModeHooks, GuestPhysMemSpace};
use super::{GuestExitInfo, GuestContext, VcpuArchState};
use crate::irq::{disable_local, DisabledLocalIrqGuard};

/// An execution object that keeps VMX operation enabled.
pub(crate) struct ArchGuestMode {}

impl ArchGuestMode {
    /// Creates a guest execution object without entering a guest.
    pub(crate) fn new() -> Result<Self> {
        Ok(Self {})
    }

    /// Attempts one guest entry and returns an exit that needs higher-level handling.
    ///
    /// Returns `None` for an internally handled exit, with local IRQs enabled.
    ///
    /// # Panics
    ///
    /// Local IRQs must be enabled.
    pub(crate) fn execute_once<H: GuestModeHooks + ?Sized>(
        &self,
        context: &mut GuestContext,
        guest_mem: &GuestPhysMemSpace,
        hooks: &H,
    ) -> Result<Option<GuestExitInfo>> {
        let guard = disable_local();
        let arch = context.arch();

        let root_paddr = guest_mem.root_paddr();
        arch.update_hgatp(root_paddr);

        hooks.pre_guest_run(arch, &guard);
        let info = arch.enter_exit();
        hooks.post_guest_run(arch, &guard);

        drop(guard);

        Ok(Some(info))
    }
}

/// A set of no-op guest mode hooks.
pub struct RiscvGuestHooks;

impl GuestModeHooks for RiscvGuestHooks {
    fn pre_guest_run(&self, state: &mut VcpuArchState, _guard: &DisabledLocalIrqGuard) {
        state.flush_interrupts();

        // Update HVIP CSR for current CPU
        state.update_hvip();
    }

    fn post_guest_run(&self, state: &mut VcpuArchState, _guard: &DisabledLocalIrqGuard) {
        state.sync_interrupts();
    }

    fn has_kernel_event(&self) -> bool {
        // We use `VcpuArchState`'s csr to determine whether a trap originated from the kernel.
        // So we do not use this func
        false
    }
}
