// SPDX-License-Identifier: MPL-2.0

//! Architecture dependent CPU-local information utilities.

pub(crate) fn get_base() -> u64 {
    let mut gp;
    // SAFETY: It is safe to read the register containing the CPU-local base.
    unsafe {
        core::arch::asm!(
            "move {gp}, $r21",
            gp = out(reg) gp,
            options(preserves_flags, nostack)
        );
    }
    gp
}
