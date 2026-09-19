// SPDX-License-Identifier: MPL-2.0

//! Hardware virtualization support for x86.

mod context;
mod types;
pub(crate) mod vmx;

/// Initializes hardware-virtualization state on the current CPU.
pub(super) fn init() {
    vmx::init_feature_control();
}

pub use self::{
    context::{GuestContext, VcpuRunState},
    types::{VcpuDtable, VcpuRegs, VcpuSegment, VcpuSregs, X86GprIndex},
};
