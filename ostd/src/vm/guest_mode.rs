// SPDX-License-Identifier: MPL-2.0

use super::GuestPhysMemSpace;
use crate::{
    arch::vm::{GuestContext, GuestExitInfo, VcpuArchState, guest_mode::ArchGuestMode},
    irq::DisabledLocalIrqGuard,
    prelude::*,
    task,
};

/// An execution mode for isolated guests.
///
/// [`Self::execute`] runs a guest vCPU until a VM exit or kernel event needs handling outside OSTD.
///
/// # Examples
///
/// ```no_run
/// # fn handle_vm_exit(reason: ostd::vm::GuestReturnReason) {}
/// #
/// use ostd::{
///     arch::vm::GuestContext,
///     prelude::*,
///     vm::{GuestMode, GuestModeHooks, GuestPhysMemSpace},
/// };
///
/// fn run_guest(
///     context: &mut GuestContext,
///     guest_mem: &GuestPhysMemSpace,
///     hooks: &impl GuestModeHooks,
/// ) -> Result<()> {
///     let guest_mode = GuestMode::new()?;
///
///     loop {
///         let return_reason = guest_mode.execute(context, guest_mem, hooks)?;
///         // Handle VM exit according to the exit reason recorded in `return_reason`.
///         handle_vm_exit(return_reason);
///     }
/// }
/// ```
pub struct GuestMode {
    arch: ArchGuestMode,
}

impl GuestMode {
    /// Creates a guest execution object without entering a guest.
    ///
    /// # Panics
    ///
    /// Panics if called in [atomic mode](crate::task::atomic_mode).
    pub fn new() -> Result<Self> {
        Ok(Self {
            arch: ArchGuestMode::new()?,
        })
    }

    /// Runs the guest until a VM exit or kernel event needs handling by the kernel client.
    ///
    /// The `hooks` inspect and update guest state before and after each guest-entry attempt.
    /// They also provide pending interrupt requests and timer deadlines through that state.
    ///
    /// # Panics
    ///
    /// Must be called in task context with local IRQs and preemption enabled.
    #[track_caller]
    pub fn execute<H: GuestModeHooks + ?Sized>(
        &self,
        context: &mut GuestContext,
        guest_mem: &GuestPhysMemSpace,
        hooks: &H,
    ) -> Result<GuestReturnReason> {
        task::atomic_mode::might_sleep();

        loop {
            task::scheduler::might_preempt();

            if let Some(exit) = self.arch.execute_once(context, guest_mem, hooks)? {
                return Ok(GuestReturnReason::VmExit(exit));
            }

            if hooks.has_kernel_event() {
                return Ok(GuestReturnReason::KernelEvent);
            }
        }
    }
}

/// Hooks called around each guest-entry attempt.
///
/// The pre-run and post-run hooks execute with local IRQs disabled and must not block.
pub trait GuestModeHooks {
    /// Prepares guest state and execution requests before they are loaded.
    fn pre_guest_run(&self, state: &mut VcpuArchState, guard: &DisabledLocalIrqGuard);

    /// Processes guest state after the host state has been restored.
    fn post_guest_run(&self, state: &mut VcpuArchState, guard: &DisabledLocalIrqGuard);

    /// Checks for a kernel event after an internally handled exit, with local IRQs enabled.
    fn has_kernel_event(&self) -> bool;
}

/// A set of no-op guest mode hooks.
pub struct DummyGuestHooks;

impl GuestModeHooks for DummyGuestHooks {
    fn pre_guest_run(&self, _state: &mut VcpuArchState, _guard: &DisabledLocalIrqGuard) {}

    fn post_guest_run(&self, _state: &mut VcpuArchState, _guard: &DisabledLocalIrqGuard) {}

    fn has_kernel_event(&self) -> bool {
        false
    }
}

/// The reason guest execution returned to the kernel client.
#[derive(Debug)]
pub enum GuestReturnReason {
    /// An exit that requires higher-level handling.
    VmExit(GuestExitInfo),
    /// A pending kernel event reported by [`GuestModeHooks::has_kernel_event`].
    KernelEvent,
}
