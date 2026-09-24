// SPDX-License-Identifier: MPL-2.0

use x86::{
    msr::{IA32_VMX_MISC, rdmsr},
    vmx::vmcs::{control, guest},
};

use super::{
    GuestContext, GuestExitInfo, GuestInterrupt, GuestTimerInstant, exit,
    host_context::HostContext,
    vmx::{VmxGuard, vmcs::Vmcs},
};
use crate::{
    Error, irq,
    prelude::*,
    task,
    vm::{GuestInterruptPort, GuestPhysMemSpace, GuestTimerPort},
};

/// An execution mode for isolated x86 guests.
///
/// `GuestMode` is the OSTD-side execution object for a guest vCPU. It enters
/// guest execution with the vCPU context and kernel-provided policy ports
/// supplied to [`Self::execute`] until a VM exit must be handled outside OSTD.
///
/// # Examples
///
/// ```no_run
/// # fn handle_vm_exit(guest_result: ostd::vm::GuestRunResult) {}
/// #
/// use ostd::{
///     arch::vm::GuestContext,
///     prelude::*,
///     vm::{GuestInterruptPort, GuestMode, GuestPhysMemSpace, GuestTimerPort},
/// };
///
/// fn run_guest(
///     context: &mut GuestContext,
///     interrupt_port: &dyn GuestInterruptPort,
///     timer_port: &dyn GuestTimerPort,
///     guest_mem: &GuestPhysMemSpace,
/// ) -> Result<()> {
///     let guest_mode = GuestMode::new()?;
///
///     loop {
///         let run_result = guest_mode.execute(context, guest_mem, interrupt_port, timer_port)?;
///         // Handle VM exit according to the exit reason recorded in `run_result`.
///         handle_vm_exit(run_result);
///     }
/// }
/// ```
pub struct GuestMode {
    vmx_guard: VmxGuard,
}

/// The reason guest execution returned to the kernel client.
#[derive(Debug)]
pub enum GuestRunResult {
    /// An exit that requires higher-level handling.
    VmExit(GuestExitInfo),
    /// A host interrupt, after which the client can reach a scheduling point.
    HostInterrupt,
}

impl GuestMode {
    /// Creates a guest execution object without entering a guest.
    pub fn new() -> Result<Self> {
        let vmx_guard = VmxGuard::acquire_vmx()?;
        Ok(Self { vmx_guard })
    }

    /// Runs the guest until an exit needs handling by the kernel client.
    ///
    /// The `interrupt_port` and `timer_port` arguments provide kernel
    /// policy for pending guest interrupts and guest timers while the vCPU
    /// is running.
    ///
    /// # Panics
    ///
    /// Must be called in task context with local IRQs enabled.
    pub fn execute<I: GuestInterruptPort + ?Sized, T: GuestTimerPort + ?Sized>(
        &self,
        context: &mut GuestContext,
        guest_mem: &GuestPhysMemSpace,
        interrupt_port: &I,
        timer_port: &T,
    ) -> Result<GuestRunResult> {
        assert!(crate::arch::irq::is_local_enabled());
        let mut host = HostContext::new();
        let _preempt_guard = task::disable_preempt();
        context.vmcs.load(&self.vmx_guard)?;

        let vmcs = &context.vmcs;
        let arch = &mut context.arch;

        {
            let irq_guard = irq::disable_local();
            // SAFETY:
            // 1. `load()` made this VMCS current, and the guards keep it current.
            // 2. The borrowed `guest_mem` ensures EPT isolation and frame lifetimes.
            unsafe {
                vmcs.setup_controls(
                    guest_mem.eptp(),
                    arch.sregs.efer,
                    &self.vmx_guard,
                    &irq_guard,
                )?;
            }
        }

        loop {
            let irq_guard = irq::disable_local();
            // SAFETY: The VMCS loaded above remains current while preemption is
            // disabled and the `context` and VMX guard are held.
            unsafe { vmcs.setup_host(&irq_guard) }?;
            // SAFETY: The same VMCS remains current under these guards.
            unsafe { vmcs.load_guest_context(arch, &self.vmx_guard, &irq_guard) }?;
            // SAFETY: The same VMCS remains current under these guards.
            let injected = unsafe { prepare_events(vmcs, interrupt_port, timer_port, &irq_guard) }?;

            host.save(&irq_guard);
            // SAFETY: The host snapshot is restored on every path below,
            // before the IRQ and preemption guards can be dropped.
            let result = unsafe { arch.load_run_state(&irq_guard) };
            // SAFETY:
            // 1. The VMCS remains current, `setup_controls` enforces EPT isolation,
            //    and `setup_host` installs this CPU's state and the exit handler.
            // 2. The `context` and `guest_mem` keep the VMCS and EPT alive;
            // 3. The code below restores software-managed host state before
            //    IRQs or preemption can resume, including on entry failure.
            let result = result.and_then(|()| unsafe { vmcs.run(&mut arch.regs, &irq_guard) });
            // SAFETY: VM exit leaves this VMCS current, and the guards remain held.
            let result = result.and_then(|()| unsafe { exit::exit_info(vmcs, &irq_guard) });
            if result.is_ok() {
                arch.save_run_state(&irq_guard);
            }
            host.load(&irq_guard);
            let exit = result?;
            // SAFETY: The VMCS remains current under the held guards.
            unsafe { vmcs.save_guest_context(arch, &irq_guard) }?;
            if let Some(interrupt) = injected {
                interrupt_port.accept_interrupt(interrupt);
            }
            drop(irq_guard);

            if let Some(result) = exit::handle_exit(exit) {
                return Ok(result);
            }
        }
    }
}

/// Prepares the guest timer and pending interrupt for VM entry.
///
/// # Safety
///
/// The VMCS must be current on this CPU.
unsafe fn prepare_events<I: GuestInterruptPort + ?Sized, T: GuestTimerPort + ?Sized>(
    vmcs: &Vmcs,
    interrupt_port: &I,
    timer_port: &T,
    irq_guard: &irq::DisabledLocalIrqGuard,
) -> Result<Option<GuestInterrupt>> {
    // SAFETY:
    // 1. The caller ensures this VMCS is current.
    // 2. The timer only bounds guest execution and does not change host state.
    unsafe {
        vmcs.write(
            guest::VMX_PREEMPTION_TIMER_VALUE,
            preemption_timer(timer_port, irq_guard) as usize,
            irq_guard,
        )
    }?;

    // Recompute injection and window exiting on every entry, including after
    // an interrupt-window exit or an earlier failed VM entry.
    // SAFETY:
    // 1. The caller ensures this VMCS is current.
    // 2. These writes do not affect host state.
    let primary = unsafe {
        vmcs.write(control::VMENTRY_INTERRUPTION_INFO_FIELD, 0, irq_guard)?;
        let primary = vmcs.read(control::PRIMARY_PROCBASED_EXEC_CONTROLS, irq_guard)?
            & !(control::PrimaryControls::INTERRUPT_WINDOW_EXITING.bits() as usize);
        vmcs.write(control::PRIMARY_PROCBASED_EXEC_CONTROLS, primary, irq_guard)?;
        primary
    };

    let Some(interrupt) = interrupt_port.query_pending_interrupt() else {
        return Ok(None);
    };
    if interrupt.vector < 32 {
        return Err(Error::InvalidArgs);
    }
    // SAFETY: The caller ensures this VMCS is current.
    let (rflags, blocking) = unsafe {
        (
            vmcs.read(guest::RFLAGS, irq_guard)?,
            vmcs.read(guest::INTERRUPTIBILITY_STATE, irq_guard)?,
        )
    };
    // Intel SDM, Vol. 3C, Section 24.4.2: STI and MOV-SS block external interrupts.
    if rflags & (1 << 9) == 0 || blocking & 3 != 0 {
        // SAFETY:
        // 1. The caller ensures this VMCS is current.
        // 2. These writes do not affect host state.
        unsafe {
            vmcs.write(
                control::PRIMARY_PROCBASED_EXEC_CONTROLS,
                primary | control::PrimaryControls::INTERRUPT_WINDOW_EXITING.bits() as usize,
                irq_guard,
            )
        }?;
        return Ok(None);
    }
    // SAFETY:
    // 1. The caller ensures this VMCS is current.
    // 2. These writes do not affect host state.
    unsafe {
        vmcs.write(
            control::VMENTRY_INTERRUPTION_INFO_FIELD,
            (1 << 31) | usize::from(interrupt.vector),
            irq_guard,
        )
    }?;
    Ok(Some(interrupt))
}

/// Converts the guest deadline to a VMX preemption-timer count.
///
/// # Safety
///
/// This CPU must support VMX.
unsafe fn preemption_timer<T: GuestTimerPort + ?Sized>(
    port: &T,
    _irq_guard: &irq::DisabledLocalIrqGuard,
) -> u32 {
    // SAFETY: The TSC is available on the supported x86-64 CPUs.
    let now = unsafe { core::arch::x86_64::_rdtsc() };
    let Some(deadline) = port.poll_deadline(GuestTimerInstant { tsc: now }) else {
        // Keep the timer enabled with its longest interval. This is finite,
        // so expiration may still return a timer exit to the client.
        return u32::MAX;
    };
    // SAFETY: The caller guarantees VMX support, so `IA32_VMX_MISC` exists.
    let rate = unsafe { rdmsr(IA32_VMX_MISC) } & 0x1f;
    let cycles = deadline.tsc.saturating_sub(now);
    u32::try_from(cycles.saturating_add((1 << rate) - 1) >> rate).unwrap_or(u32::MAX)
}
