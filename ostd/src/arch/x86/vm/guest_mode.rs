// SPDX-License-Identifier: MPL-2.0

use x86::{
    msr,
    vmx::vmcs::{control, guest},
};
use x86_64::{VirtAddr, instructions::tables::lgdt, structures::DescriptorTablePointer};

use super::{
    GuestContext, GuestExitInfo, VcpuArchState, VmxExitReason, exit,
    vmx::{VmxGuard, vmcs::CurrentVmcs},
};
use crate::{
    Error,
    arch::{
        self,
        cpu::context::GsBase,
        trap::{gdt, syscall},
    },
    irq::DisabledLocalIrqGuard,
    prelude::*,
    vm::{GuestModeHooks, GuestPhysMemSpace},
};

/// An execution object that keeps VMX operation enabled.
pub(crate) struct ArchGuestMode {
    vmx_guard: VmxGuard,
}

enum ArmedEvent {
    PendingException,
    PendingInterrupt,
}

impl ArchGuestMode {
    /// Creates a guest execution object without entering a guest.
    pub(crate) fn new() -> Result<Self> {
        Ok(Self {
            vmx_guard: VmxGuard::acquire_vmx()?,
        })
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
        let (arch, vmcs) = context.arch_and_vmcs();

        let mut irq_guard = None;
        let current_vmcs = vmcs.load(&self.vmx_guard, &mut irq_guard)?;
        hooks.pre_guest_run(arch, current_vmcs.irq_guard());
        let result = current_vmcs.run_guest(arch, guest_mem, &self.vmx_guard);
        hooks.post_guest_run(arch, current_vmcs.irq_guard());
        let exit = result?;

        // `ACK_INTERRUPT_ON_EXIT` is clear, so enabling IRQs delivers
        // pending host interrupts through the normal IRQ path.
        drop(current_vmcs);
        drop(irq_guard);

        match VmxExitReason::try_from(exit.exit_reason) {
            Ok(VmxExitReason::EXTERNAL_INTERRUPT | VmxExitReason::INTERRUPT_WINDOW) => Ok(None),
            _ => Ok(Some(exit)),
        }
    }
}

impl CurrentVmcs<'_> {
    /// Runs a guest and restores host state before returning, including on errors.
    fn run_guest(
        &self,
        arch: &mut VcpuArchState,
        guest_mem: &GuestPhysMemSpace,
        vmx_guard: &VmxGuard,
    ) -> Result<GuestExitInfo> {
        // SAFETY: The borrowed `guest_mem` ensures EPT isolation and frame lifetimes.
        unsafe { self.sync_controls(guest_mem.eptp(), arch.sregs.efer) }?;
        self.load_guest_context(arch, vmx_guard)?;
        let armed_event = self.prepare_events(arch)?;
        self.sync_host()?;

        let kernel_gs_base_restorer = KernelGsBaseRestorer::new(self.irq_guard());
        // SAFETY: `syscall_msrs_restorer` is always dropped below.
        let syscall_msrs_restorer = unsafe { arch.load_run_state(self.irq_guard()) }?;

        // SAFETY:
        // 1. The controls and host state have been installed in this current VMCS.
        // 2. `self` and `guest_mem` keep the VMCS and EPT resources alive.
        unsafe { self.run(&mut arch.regs) }?;
        let exit = exit::exit_info(self)?;

        arch.save_run_state(self.irq_guard());

        drop(syscall_msrs_restorer);
        drop(kernel_gs_base_restorer);

        match armed_event {
            Some(ArmedEvent::PendingException) => arch.pending_exception = None,
            Some(ArmedEvent::PendingInterrupt) => arch.pending_interrupt = None,
            None => {}
        }

        arch.idt_vectoring_info = exit.idt_vectoring_info;
        arch.idt_vectoring_error_code = exit.idt_vectoring_error_code;

        self.save_guest_context(arch)?;

        Ok(exit)
    }

    /// Prepares events and returns whether a new external interrupt is armed.
    fn prepare_events(&self, arch: &mut VcpuArchState) -> Result<Option<ArmedEvent>> {
        const VALID: u32 = 1 << 31;
        const DELIVER_ERROR_CODE: u32 = 1 << 11;
        const HARDWARE_EXCEPTION: u32 = 3 << 8;

        let mut timer_count = if let Some(deadline) = arch.timer_deadline {
            // SAFETY: `self` keeps VMX enabled, so `IA32_VMX_MISC` exists.
            let rate = unsafe { msr::rdmsr(msr::IA32_VMX_MISC) } & 0x1f;
            let cycles = deadline.tsc.saturating_sub(arch::read_tsc());
            u32::try_from(cycles.saturating_add((1 << rate) - 1) >> rate).unwrap_or(u32::MAX)
        } else {
            u32::MAX
        };
        let vectoring = arch.idt_vectoring_info;
        let vectoring_type = (vectoring & VALID != 0).then_some((vectoring >> 8) & 7);
        // let rflags = self.read(guest::RFLAGS)?;
        // let blocking = self.read(guest::INTERRUPTIBILITY_STATE)?;
        // let is_interruptible = rflags & (1 << 9) != 0 && blocking & 3 == 0;

        let (exception_vector, is_pending_exception) = if matches!(vectoring_type, Some(3..=6)) {
            (Some((vectoring & 0xff, arch.idt_vectoring_error_code)), false)
        } else {
            (arch.pending_exception, true)
        };

        let (interrupt_vector, is_pending_interrupt) = if matches!(vectoring_type, Some(0)) {
            (Some(vectoring as u8), false)
        } else {
            (arch.pending_interrupt, true)
        };

        let irq_window = control::PrimaryControls::INTERRUPT_WINDOW_EXITING.bits() as usize;
        let mut primary = self.read(control::PRIMARY_PROCBASED_EXEC_CONTROLS)?;
        primary &= !irq_window;
        if interrupt_vector.is_some() {
            if exception_vector.is_some() {
                if is_interruptible {
                    timer_count = 0;
                } else {
                    primary |= irq_window;
                }
            } else {
                if !is_interruptible {
                    primary |= irq_window;
                } else if !is_pending_interrupt && arch.pending_interrupt.is_some() {
                    timer_count = 0;
                }
            }
        }
        // SAFETY: This only bounds guest execution.
        unsafe { self.write(guest::VMX_PREEMPTION_TIMER_VALUE, timer_count as usize) }?;
        // SAFETY: This only enables exits when the guest can accept an interrupt.
        unsafe { self.write(control::PRIMARY_PROCBASED_EXEC_CONTROLS, primary) }?;

        let (info, error_code, armed_event) = if let Some((vector, error_code)) = exception_vector {
            let mut info = VALID | HARDWARE_EXCEPTION | u32::from(vector);
            if arch.sregs.cr0 & 1 != 0 && matches!(vector, 8 | 10..=14 | 17 | 21) {
                info |= DELIVER_ERROR_CODE;
            }
            (info, error_code, is_pending_exception.then(|| ArmedEvent::PendingException))
        } else if is_interruptible && let Some(vector) = interrupt_vector {
            (VALID | u32::from(vector), 0, is_pending_interrupt.then(|| ArmedEvent::PendingInterrupt))
        } else {
            (0, 0, None)
        };

        // SAFETY: This only provides details about the event to be injected into the guest.
        unsafe {
            if info & DELIVER_ERROR_CODE != 0 {
                self.write(control::VMENTRY_EXCEPTION_ERR_CODE, error_code as usize)?;
            }
            self.write(control::VMENTRY_INTERRUPTION_INFO_FIELD, info as usize)?;
        }

        Ok(armed_event)
    }
}

struct KernelGsBaseRestorer<'a> {
    kernel_gs_base: GsBase,
    irq_guard: &'a DisabledLocalIrqGuard,
}

impl<'a> KernelGsBaseRestorer<'a> {
    fn new(irq_guard: &'a DisabledLocalIrqGuard) -> Self {
        let mut kernel_gs_base = GsBase::default();
        kernel_gs_base.save(irq_guard);
        Self {
            kernel_gs_base,
            irq_guard,
        }
    }
}

impl<'a> Drop for KernelGsBaseRestorer<'a> {
    fn drop(&mut self) {
        self.kernel_gs_base.load(self.irq_guard);
    }
}

pub(super) struct SyscallMsrsRestorer<'a> {
    irq_guard: &'a DisabledLocalIrqGuard,
}

impl<'a> SyscallMsrsRestorer<'a> {
    pub(super) fn new(irq_guard: &'a DisabledLocalIrqGuard) -> Self {
        Self { irq_guard }
    }
}

impl<'a> Drop for SyscallMsrsRestorer<'a> {
    fn drop(&mut self) {
        // VM exits set GDTR limits to 0xffff, so reload the host GDTR.
        let gdtr = DescriptorTablePointer {
            limit: gdt::gdt_limit(),
            base: VirtAddr::new(gdt::gdt_base(self.irq_guard) as u64),
        };
        // SAFETY: The GDT is valid to load because it is the correct one for the current CPU.
        unsafe { lgdt(&gdtr) };

        syscall::configure_msrs(self.irq_guard);
    }
}
