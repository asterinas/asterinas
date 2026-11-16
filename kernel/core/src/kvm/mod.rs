// SPDX-License-Identifier: MPL-2.0

pub(crate) mod run_page;
pub(crate) mod vcpu;
pub(crate) mod vm;
mod vm_memory;

#[cfg_attr(target_arch = "x86_64", path = "arch/x86/mod.rs")]
mod arch;
