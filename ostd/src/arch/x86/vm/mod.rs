// SPDX-License-Identifier: MPL-2.0

//! Hardware virtualization support for x86.

mod context;
pub(crate) mod ept;
mod exit;
pub(crate) mod guest_mode;
mod types;
mod vmcs;
pub(crate) mod vmx;
mod x86;

/// Initializes hardware-virtualization state on the current CPU.
pub(super) fn init() {
    vmx::init_feature_control();
}

pub use self::{
    context::{GuestContext, VcpuArchState},
    exit::{GuestExitInfo, VmxExitReason},
    types::{
        GuestInterrupt, GuestTimerInstant, VcpuDescTable, VcpuMsrs, VcpuRegs, VcpuSegment,
        VcpuSregs,
    },
};
