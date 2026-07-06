// SPDX-License-Identifier: MPL-2.0

//! Providing the ability to exit QEMU and return a value as debug result.

/// The exit code of QEMU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QemuExitCode {
    /// The code that indicates a successful exit.
    Success,
    /// The code that indicates a failed exit.
    Failed,
}

/// Exit QEMU with the given exit code.
pub fn exit_qemu(exit_code: QemuExitCode) -> ! {
    log::debug!("exit qemu with exit code {exit_code:?}");
    // Use legacy SBI system reset (EID=0x53525354) instead of
    // sbi-rt binary interface to avoid spec version mismatch.
    let reset_type: usize = match exit_code {
        QemuExitCode::Success => 0, // shutdown
        QemuExitCode::Failed => 1,  // cold reboot
    };
    unsafe {
        core::arch::asm!(
            "li a7, 0x53525354",
            "ecall",
            in("a0") 0usize,          // reset_type = shutdown
            in("a1") reset_type,       // reset_reason
        );
    }
    unreachable!("qemu does not exit");
}
