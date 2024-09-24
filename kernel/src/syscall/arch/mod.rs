// SPDX-License-Identifier: MPL-2.0

//! Implement the `syscall_dispatch` function and the const values of system call number such as `SYS_READ`.

#[cfg(target_arch = "riscv64")]
pub mod riscv;
#[cfg(target_arch = "x86_64")]
pub mod x86;

#[cfg(target_arch = "riscv64")]
pub use self::riscv::*;
#[cfg(target_arch = "x86_64")]
pub use self::x86::*;
