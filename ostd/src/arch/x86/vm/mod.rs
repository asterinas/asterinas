// SPDX-License-Identifier: MPL-2.0

//! Hardware virtualization support for x86.
mod context;
pub(crate) mod ept;
mod types;
pub(crate) mod vmx;

/// Initializes hardware-virtualization state on the current CPU.
pub(super) fn init() {
    vmx::init_feature_control();
    vmx::invept::init_ept_support();
}

pub use self::{
    context::GuestContext,
    types::{VcpuDescTable, VcpuMsrs, VcpuRegs, VcpuSegment, VcpuSregs},
};
