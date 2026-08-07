// SPDX-License-Identifier: MPL-2.0

//! Hardware virtualization support for x86.

pub(crate) mod vmx;

#[cfg(all(ktest, feature = "vmx_ktest"))]
mod tests;
