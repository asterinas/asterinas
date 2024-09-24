// SPDX-License-Identifier: MPL-2.0

//! Platform-specific code.
//!
//! Each architecture that Asterinas supports may contain a submodule here.

#[cfg(target_arch = "riscv64")]
pub mod riscv;
#[cfg(target_arch = "x86_64")]
pub mod x86;

#[cfg(target_arch = "riscv64")]
pub use self::riscv::*;
#[cfg(target_arch = "x86_64")]
pub use self::x86::*;
