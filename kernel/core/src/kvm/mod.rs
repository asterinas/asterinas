// SPDX-License-Identifier: MPL-2.0
//! The concrete implementation of KVM (architecture-independent portion), 
//! encompassing specific logic for vCPUs, VMs, memory, etc.

mod vm;
mod vcpu;
mod vm_memory;
mod ioctl;
mod run_page;

#[cfg_attr(target_arch = "riscv64", path = "arch/riscv/mod.rs")]
mod arch;

pub use vm::Vm;
pub use ioctl::{check_extension, UserMemoryRegion, OneReg};
pub use vcpu::Vcpu;
pub use run_page::KvmRun;

pub enum KvmRegSize {
    U32,
    U64,
}
