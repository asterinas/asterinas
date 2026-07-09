// SPDX-License-Identifier: MPL-2.0

//! CPU feature detection.
//!
//! TODO: Parse `ID_AA64ISAR*`/`ID_AA64PFR*` to detect optional features
//! (e.g. the RNG extension, pointer authentication, SVE).

/// Enables architecture-specific CPU features on the current processor.
pub(crate) fn init() {
    // Enable access to the Advanced SIMD and floating-point registers at EL1/EL0
    // by clearing `CPACR_EL1.FPEN` traps. Even on the softfloat kernel target,
    // user programs may use FP/SIMD.
    // SAFETY: Programming `CPACR_EL1` to permit FP/SIMD access is safe.
    unsafe {
        core::arch::asm!(
            "mrs {tmp}, cpacr_el1",
            "orr {tmp}, {tmp}, #(0b11 << 20)", // FPEN = 0b11: no trapping
            "msr cpacr_el1, {tmp}",
            "isb",
            tmp = out(reg) _,
            options(nostack, preserves_flags),
        );
    }
}
