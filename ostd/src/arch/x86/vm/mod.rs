// SPDX-License-Identifier: MPL-2.0

//! Hardware virtualization support for x86.

pub(crate) mod vmx;

/// Initializes hardware-virtualization state on the current CPU.
pub(super) fn init() {
    vmx::init_feature_control();
}
