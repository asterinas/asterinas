// SPDX-License-Identifier: MPL-2.0

use super::{
    csr::{Htinst, Htval},
    vcpu::VcpuArchState,
};

/// A structure that stores relevant information after a vCPU exits and returns to the host.
#[derive(Clone, Copy, Debug, Default)]
pub struct GuestExitInfo {
    sepc: usize,
    scause: usize,
    stval: usize,
    htval: usize,
    htinst: usize,
}

impl GuestExitInfo {
    /// Get the virtual instruction.
    pub fn virtual_instruction(&self) -> usize {
        self.stval
    }

    /// Get the trap instruction.
    pub fn trap_instruction(&self) -> usize {
        self.htinst
    }

    /// Get the G-stage page fault address.
    pub fn fault_addr(&self) -> usize {
        (self.htval << 2) | (self.stval & 0x3)
    }

    /// Get the type of trap.
    pub fn trap_type(&self) -> usize {
        self.scause
    }
}

impl VcpuArchState {
    /// Get `GuestExitInfo` from CSRs.
    pub(super) fn exit_info(&self) -> GuestExitInfo {
        GuestExitInfo {
            sepc: self.guest_context.sepc,
            scause: riscv::register::scause::read().bits(),
            stval: riscv::register::stval::read(),
            // SAFETY: Ensure that a trap has occurred prior to the call;
            // the hardware will have automatically written the information into the registers.
            htval: unsafe { Htval::read() },
            // SAFETY: Ensure that a trap has occurred prior to the call;
            // the hardware will have automatically written the information into the registers.
            htinst: unsafe { Htinst::read() },
        }
    }
}
