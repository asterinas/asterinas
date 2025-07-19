// SPDX-License-Identifier: MPL-2.0

//! Architecture dependent CPU-local information utilities.

/// Acquire the base pointer for the current CPU, which is used to access CPU-local variables.
pub(crate) fn get_base() -> u64 {
    let mut gp;
    unsafe {
        core::arch::asm!(
            "move {gp}, $r21",
            gp = out(reg) gp,
            options(preserves_flags, nostack)
        );
    }
    gp
}
