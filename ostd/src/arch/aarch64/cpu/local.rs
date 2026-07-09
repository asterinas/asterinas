// SPDX-License-Identifier: MPL-2.0

//! Architecture dependent CPU-local information utilities.
//!
//! On AArch64 the CPU-local storage base is held in `TPIDR_EL1`, initialised by
//! the boot assembly to point at `__cpu_local_start`.

pub(crate) fn get_base() -> u64 {
    let base;
    // SAFETY: Reading `TPIDR_EL1` has no side effects.
    unsafe {
        core::arch::asm!(
            "mrs {base}, tpidr_el1",
            base = out(reg) base,
            options(preserves_flags, nostack, nomem),
        );
    }
    base
}
