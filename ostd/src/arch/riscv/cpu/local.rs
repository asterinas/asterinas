// SPDX-License-Identifier: MPL-2.0

//! Architecture dependent CPU-local information utilities.

pub(crate) fn get_base() -> u64 {
    let mut gp;
    unsafe {
        core::arch::asm!(
            "mv {gp}, gp",
            gp = out(reg) gp,
            options(preserves_flags, nostack)
        );
    }
    gp
}
