// SPDX-License-Identifier: MPL-2.0

//! Provides the implementation of the poweroff functionality.

use crate::arch::qemu::{exit_qemu, QemuExitCode};

/// Powers off the system.
pub fn poweroff() -> ! {
    // TODO: Implement the poweroff behavior on a real machine.
    exit_qemu(QemuExitCode::Success);
}
