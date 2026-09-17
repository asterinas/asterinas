// SPDX-License-Identifier: MPL-2.0

use x86::msr::{IA32_FMASK, IA32_KERNEL_GSBASE, IA32_LSTAR, IA32_STAR, rdmsr, wrmsr};
use x86_64::{VirtAddr, registers::control::Cr2};

use super::{
    VcpuMsrs, VcpuRegs, VcpuSregs,
    vmx::{VmxGuard, vmcs::Vmcs},
    x86::write_cr2_raw,
};
use crate::{Error, irq::DisabledLocalIrqGuard, prelude::*};

/// The guest-visible architectural state of an x86 vCPU.
///
/// Register values can be modified without affecting the host CPU. The caller
/// is responsible for initializing them for the guest's execution mode.
pub struct GuestContext {
    arch: VcpuArchState,
    vmcs: Vmcs,
}

impl GuestContext {
    /// Creates a guest vCPU context from initialized register state.
    ///
    /// # Panics
    ///
    /// Local IRQs must be enabled.
    pub fn new(regs: VcpuRegs, sregs: VcpuSregs, msrs: VcpuMsrs) -> Result<Self> {
        let vmx_guard = VmxGuard::acquire_vmx()?;
        Ok(Self {
            arch: VcpuArchState { regs, sregs, msrs },
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

pub(super) struct VcpuArchState {
    pub regs: VcpuRegs,
    pub sregs: VcpuSregs,
    pub msrs: VcpuMsrs,
}

impl VcpuArchState {
    /// Loads guest state not handled by VMX.
    ///
    /// # Safety
    ///
    /// The host MSRs must be restored on this CPU before enabling IRQs or
    /// scheduling, including if guest entry fails.
    pub(super) unsafe fn load_run_state(&self, _irq_guard: &DisabledLocalIrqGuard) -> Result<()> {
        // These values are loaded by WRMSR in host mode, outside VM-entry's
        // guest-state checks. Reject values that would cause a host #GP.
        for value in [self.msrs.kernel_gs_base, self.msrs.lstar] {
            if VirtAddr::try_new(value).is_err() {
                return Err(Error::InvalidArgs);
            }
        }
        if self.msrs.syscall_mask > u32::MAX as u64 {
            return Err(Error::InvalidArgs);
        }

        write_cr2_raw(self.sregs.cr2);
        // SAFETY: These MSRs are architectural on x86-64. The checks above
        // ensure canonical addresses and no reserved FMASK bits; STAR has no
        // reserved bits. The caller arranges restoration of the host state.
        unsafe {
            wrmsr(IA32_STAR, self.msrs.star);
            wrmsr(IA32_LSTAR, self.msrs.lstar);
            wrmsr(IA32_FMASK, self.msrs.syscall_mask);
            wrmsr(IA32_KERNEL_GSBASE, self.msrs.kernel_gs_base);
        }
        Ok(())
    }

    /// Saves guest state not handled by VMX.
    pub(super) fn save_run_state(&mut self, _irq_guard: &DisabledLocalIrqGuard) {
        self.sregs.cr2 = Cr2::read_raw();
        // SWAPGS can change KERNEL_GSBASE without an MSR-access exit.
        // SAFETY: IA32_KERNEL_GSBASE is present on x86-64 and reading it has no side effects.
        self.msrs.kernel_gs_base = unsafe { rdmsr(IA32_KERNEL_GSBASE) };
    }
}
