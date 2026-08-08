// SPDX-License-Identifier: MPL-2.0

use super::{VcpuMsrs, VcpuRegs, VcpuSregs};

/// The guest-visible architectural state of an x86 vCPU.
///
/// Register values can be modified without affecting the host CPU. The caller
/// is responsible for initializing them for the guest's execution mode.
pub struct GuestContext {
    regs: VcpuRegs,
    sregs: VcpuSregs,
    msrs: VcpuMsrs,
}

impl GuestContext {
    /// Creates a guest vCPU context from initialized register state.
    pub fn new(regs: VcpuRegs, sregs: VcpuSregs, msrs: VcpuMsrs) -> Self {
        Self { regs, sregs, msrs }
    }

    /// Returns the guest's general-purpose registers, instruction pointer, and flags.
    pub fn regs(&self) -> &VcpuRegs {
        &self.regs
    }

    /// Returns a mutable reference to the guest's general-purpose registers,
    /// instruction pointer, and flags.
    pub fn regs_mut(&mut self) -> &mut VcpuRegs {
        &mut self.regs
    }

    /// Returns the guest's special registers.
    pub fn sregs(&self) -> &VcpuSregs {
        &self.sregs
    }

    /// Returns a mutable reference to the guest's special registers.
    pub fn sregs_mut(&mut self) -> &mut VcpuSregs {
        &mut self.sregs
    }

    /// Returns the guest's MSRs that are not part of [`VcpuSregs`].
    pub fn msrs(&self) -> &VcpuMsrs {
        &self.msrs
    }

    /// Returns a mutable reference to the guest's MSRs that are not part of [`VcpuSregs`].
    pub fn msrs_mut(&mut self) -> &mut VcpuMsrs {
        &mut self.msrs
    }
}
