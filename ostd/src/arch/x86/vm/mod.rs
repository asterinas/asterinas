// SPDX-License-Identifier: MPL-2.0

//! Hardware virtualization support for x86.

mod context;
pub(crate) mod ept;
mod exit;
mod guest_mode;
mod host_context;
mod types;
mod vmcs;
pub(crate) mod vmx;
mod x86;

/// Initializes hardware-virtualization state on the current CPU.
pub(super) fn init() {
    use crate::arch::cpu::extension::{IsaExtensions, has_extensions};
    vmx::init_feature_control();
    if has_extensions(IsaExtensions::VMX) {
        vmx::invept::check_invept_support();
    }
}

pub use self::{
    context::{GuestContext, VcpuRunState},
    exit::{GuestExitInfo, VmxExitReason},
    guest_mode::{GuestMode, GuestRunResult},
    types::{
        GuestInterrupt, GuestTimerInstant, VcpuDtable, VcpuRegs, VcpuSegment, VcpuSregs,
        X86GprIndex,
    },
};
