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
    vmx::init_feature_control();
    vmx::invept::init_ept_support();
}

pub use self::{
    context::GuestContext,
    exit::{GuestExitInfo, VmxExitReason},
    guest_mode::{GuestMode, GuestRunResult},
    types::{
        GuestInterrupt, GuestTimerInstant, VcpuDescTable, VcpuMsrs, VcpuRegs, VcpuSegment,
        VcpuSregs,
    },
};
