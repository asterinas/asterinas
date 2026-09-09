// SPDX-License-Identifier: MPL-2.0

//! Hardware virtualization support for x86.

pub(crate) mod ept;
pub(crate) mod vmx;

/// Initializes hardware-virtualization state on the current CPU.
pub(super) fn init() {
    use crate::arch::cpu::extension::{IsaExtensions, has_extensions};
    vmx::init_feature_control();
    if has_extensions(IsaExtensions::VMX) {
        vmx::invept::check_invept_support();
    }
}
