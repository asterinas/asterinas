// SPDX-License-Identifier: MPL-2.0

//! Provides the implementation of the poweroff functionality.

use core::arch::x86_64::__cpuid;

use super::qemu::{exit_qemu, QemuExitCode};

/// Checks if the system is running in QEMU.
///
/// This function uses CPUID to detect QEMU hypervisor signature.
fn is_running_in_qemu() -> bool {
    // SAFETY: CPUID is always safe to call
    let result = unsafe { __cpuid(0x40000000) };

    let mut signature = [0u8; 12];
    signature[0..4].copy_from_slice(&result.ebx.to_ne_bytes());
    signature[4..8].copy_from_slice(&result.ecx.to_ne_bytes());
    signature[8..12].copy_from_slice(&result.edx.to_ne_bytes());

    // Check for QEMU hypervisor signature: "TCGTCGTCGTCG" or "KVMKVMKVM"
    // Reference: https://wiki.osdev.org/QEMU_fw_cfg
    signature == *b"TCGTCGTCGTCG" || signature.starts_with(b"KVMKVMKVM")
}

/// Powers off the system.
pub fn poweroff() -> ! {
    if is_running_in_qemu() {
        exit_qemu(QemuExitCode::Success);
    }

    todo!("Implement ACPI shutdown");
}
