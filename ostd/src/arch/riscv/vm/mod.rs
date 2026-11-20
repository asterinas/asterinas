// SPDX-License-Identifier: MPL-2.0

//! Virtual machine (VM) support for RISC-V.

mod context;
mod cpu;
mod csr;
mod exit;
mod timer;
mod types;
mod vcpu;

pub use context::{GuestContext, KvmVcpuCsr, VcpuContext};
pub use cpu::{disable_virtualization_cpu, enable_virtualization_cpu};
pub use exit::GuestExitInfo;
pub use types::{
    CAUSE_IRQ_FLAG, HGATP_MODE_48, HGATP_MODE_SHIFT, IRQ_LOCAL_MASK, VSIP_TO_HVIP_SHIFT,
    VSIP_VALID_MASK,
};
pub use vcpu::VcpuArchState;
