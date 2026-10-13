// SPDX-License-Identifier: MPL-2.0

//! VM (Virtual Machine Extensions) support for riscv.

pub(crate) mod gstage;
mod context;

mod types;

mod csr;
mod timer;
mod cpu;
mod onereg;
mod vcpu;
mod isa;
mod sbi;
mod insn;

pub use vcpu::{KvmCpuTrap, VcpuArchInner};
pub use insn::{KvmInsnReturn, handle_system_opcode_insn, insn_len};
pub use csr::HSTATUS_SPV;
pub use cpu::enable_virtualization_cpu;
pub use context::VcpuContext;
