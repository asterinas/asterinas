// SPDX-License-Identifier: MPL-2.0

//! Provides the implementation of the poweroff functionality.

use crate::arch::{
    cpu::cpuid::query_is_running_in_qemu,
    qemu::{exit_qemu, QemuExitCode},
};

/// Powers off the system.
pub fn poweroff() -> ! {
    if query_is_running_in_qemu() {
        exit_qemu(QemuExitCode::Success);
    }

    todo!("Implement ACPI shutdown");
}
