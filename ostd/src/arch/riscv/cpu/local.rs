// SPDX-License-Identifier: MPL-2.0

//! Architecture dependent CPU-local information utilities.

pub(crate) unsafe fn set_base(addr: u64) {
    core::arch::asm!(
        "mv gp, {addr}",
        addr = in(reg) addr,
        options(preserves_flags, nostack)
    );
}

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
