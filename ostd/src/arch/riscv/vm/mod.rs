// SPDX-License-Identifier: MPL-2.0

//! VM (Virtual Machine Extensions) support for riscv.

mod context;
pub(crate) mod gstage;

mod types;

mod context_switch;
mod cpu;
mod csr;
mod exit;
mod guest_mode;
mod isa;
mod timer;

pub use cpu::enable_virtualization_cpu;
pub use csr::HSTATUS_SPV;
pub(crate) use guest_mode::ArchGuestMode;

pub use self::{
    context::{GuestContext, VcpuArchState},
    exit::{GuestException, GuestExitInfo, INTERRUPT_BIT},
    types::VcpuRegs,
};
