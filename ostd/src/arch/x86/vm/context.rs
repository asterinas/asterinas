// SPDX-License-Identifier: MPL-2.0

use x86::msr::{IA32_FMASK, IA32_LSTAR, IA32_STAR, wrmsr};
use x86_64::{VirtAddr, registers::control::Cr2};

use super::{
    GuestInterrupt, GuestTimerInstant, VcpuMsrs, VcpuRegs, VcpuSregs,
    guest_mode::SyscallMsrsRestorer,
    vmx::{VmxGuard, vmcs::Vmcs},
    x86::write_cr2_raw,
};
use crate::{Error, arch::cpu::context::GsBase, irq::DisabledLocalIrqGuard, prelude::*};

/// The guest-visible architectural state of an x86 vCPU.
///
/// Register values can be modified without affecting the host CPU. The caller
/// is responsible for initializing them for the guest's execution mode.
///
/// This context must be dropped outside [atomic mode](crate::task::atomic_mode).
pub struct GuestContext {
    arch: VcpuArchState,
    vmcs: Vmcs,
}

impl GuestContext {
    /// Creates a guest vCPU context from initialized register state.
    ///
    /// # Panics
    ///
    /// Panics if called in [atomic mode](crate::task::atomic_mode).
    pub fn new(regs: VcpuRegs, sregs: VcpuSregs, msrs: VcpuMsrs) -> Result<Self> {
        let vmx_guard = VmxGuard::acquire_vmx()?;
        Ok(Self {
            arch: VcpuArchState {
                regs,
                sregs,
                msrs,
                pending_interrupt: None,
                timer_deadline: None,
            },
            vmcs: Vmcs::new(&vmx_guard)?,
        })
    }

    /// Returns the guest's general-purpose registers, instruction pointer, and flags.
    pub fn regs(&self) -> &VcpuRegs {
        &self.arch.regs
    }

    /// Returns a mutable reference to the guest's general-purpose registers,
    /// instruction pointer, and flags.
    pub fn regs_mut(&mut self) -> &mut VcpuRegs {
        &mut self.arch.regs
    }

    /// Returns the guest's special registers.
    pub fn sregs(&self) -> &VcpuSregs {
        &self.arch.sregs
    }

    /// Returns a mutable reference to the guest's special registers.
    pub fn sregs_mut(&mut self) -> &mut VcpuSregs {
        &mut self.arch.sregs
    }

    /// Returns the guest's MSRs that are not part of [`VcpuSregs`].
    pub fn msrs(&self) -> &VcpuMsrs {
        &self.arch.msrs
    }

    /// Returns a mutable reference to the guest's MSRs that are not part of [`VcpuSregs`].
    pub fn msrs_mut(&mut self) -> &mut VcpuMsrs {
        &mut self.arch.msrs
    }

    /// Returns the mutable architectural state and the associated VMCS.
    pub(super) fn arch_and_vmcs(&mut self) -> (&mut VcpuArchState, &Vmcs) {
        (&mut self.arch, &self.vmcs)
    }
}

impl Drop for GuestContext {
    fn drop(&mut self) {
        if let Err(err) =
            VmxGuard::acquire_vmx().and_then(|vmx_guard| self.vmcs.deactivate(&vmx_guard))
        {
            // The active set retains the VMCS until a later clear succeeds.
            error!("failed to clear VMCS during context destruction: {:?}", err);
        }
    }
}

/// The register state and execution requests of an x86 vCPU.
pub struct VcpuArchState {
    /// The general-purpose registers, instruction pointer, and flags.
    pub regs: VcpuRegs,
    /// The special registers and segment state.
    pub sregs: VcpuSregs,
    /// The model-specific registers.
    pub msrs: VcpuMsrs,
    /// An external interrupt awaiting injection by OSTD.
    pub pending_interrupt: Option<GuestInterrupt>,
    /// The guest TSC deadline for returning from guest execution.
    pub timer_deadline: Option<GuestTimerInstant>,
}

impl VcpuArchState {
    /// Loads guest state not handled by VMX.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the returned restorer is not forgotten.
    pub(super) unsafe fn load_run_state<'a>(
        &self,
        irq_guard: &'a DisabledLocalIrqGuard,
    ) -> Result<SyscallMsrsRestorer<'a>> {
        // These values are loaded in host mode, outside VM-entry's guest-state
        // checks. Reject values that would cause a host #GP.
        for value in [self.msrs.kernel_gs_base, self.msrs.lstar] {
            if VirtAddr::try_new(value).is_err() {
                return Err(Error::InvalidArgs);
            }
        }
        if self.msrs.syscall_mask > u32::MAX as u64 {
            return Err(Error::InvalidArgs);
        }

        let syscall_msrs_restorer = SyscallMsrsRestorer::new(irq_guard);
        // SAFETY: These MSRs are architectural on x86-64. The checks above
        // ensure canonical addresses and no reserved `IA32_FMASK` bits;
        // `IA32_STAR` has no reserved bits. The caller restores the host state.
        unsafe {
            wrmsr(IA32_STAR, self.msrs.star);
            wrmsr(IA32_LSTAR, self.msrs.lstar);
            wrmsr(IA32_FMASK, self.msrs.syscall_mask);
        }

        write_cr2_raw(self.sregs.cr2);
        GsBase::new(self.msrs.kernel_gs_base as usize).load(irq_guard);
        Ok(syscall_msrs_restorer)
    }

    /// Saves guest state not handled by VMX.
    pub(super) fn save_run_state(&mut self, irq_guard: &DisabledLocalIrqGuard) {
        self.sregs.cr2 = Cr2::read_raw();
        // `SWAPGS` can change `IA32_KERNEL_GSBASE` without an MSR-access exit.
        self.msrs.kernel_gs_base = {
            let mut kernel_gs_base = GsBase::default();
            kernel_gs_base.save(irq_guard);
            kernel_gs_base.addr() as u64
        };
    }
}
