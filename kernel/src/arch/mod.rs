// SPDX-License-Identifier: MPL-2.0

#[cfg(target_arch = "x86_64")]
mod x86;
#[cfg(target_arch = "x86_64")]
pub use x86::*;
