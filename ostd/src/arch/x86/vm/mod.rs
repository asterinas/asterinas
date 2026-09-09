// SPDX-License-Identifier: MPL-2.0

//! Guest-visible x86 CPU state.

mod context;
mod types;

pub use self::{
    context::{GuestContext, VcpuRunState},
    types::{VcpuDtable, VcpuRegs, VcpuSegment, VcpuSregs, X86GprIndex},
};

#[cfg(ktest)]
mod tests;
