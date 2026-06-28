// SPDX-License-Identifier: MPL-2.0

//! Provides low-level Intel VMX support.
//!
//! This module contains VMX instruction wrappers, VMCS field definitions,
//! control-register shadow helpers, and x86 register-state types. Higher-level
//! guest execution, VM-exit handling, and device-facing virtualization APIs are
//! intentionally kept outside this foundation layer.

#![expect(
    dead_code,
    reason = "This foundational VMX layer is staged before higher-level guest execution code uses it."
)]

pub(crate) mod control_regs;
mod types;
pub(crate) mod vmcs;
pub(crate) mod vmx;
pub(crate) mod x86;

#[cfg(ktest)]
mod tests;
