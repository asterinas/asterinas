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
        Ok(Some(GuestExitInfo::default()))
    }
}

/// A set of no-op guest mode hooks.
pub struct RiscvGuestHooks;

impl GuestModeHooks for RiscvGuestHooks {
    fn pre_guest_run(&self, state: &mut VcpuArchState, _guard: &DisabledLocalIrqGuard) {
        
    }

    fn post_guest_run(&self, state: &mut VcpuArchState, _guard: &DisabledLocalIrqGuard) {
        
    }

    fn has_kernel_event(&self) -> bool {
        false
    }
}
