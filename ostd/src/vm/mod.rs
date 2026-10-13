// SPDX-License-Identifier: MPL-2.0

//! Guest execution and physical memory management.

#[cfg(target_arch = "x86_64")]
pub mod gpm_space;
#[cfg(target_arch = "x86_64")]
pub use gpm_space::GuestPhysMemSpace;

#[cfg(target_arch = "riscv64")]
pub mod temp_riscv_gpm;
#[cfg(target_arch = "riscv64")]
pub use temp_riscv_gpm::GuestPhysMemSpace;

/// A guest physical address.
pub type Gpaddr = usize;
