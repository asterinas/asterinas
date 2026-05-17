// SPDX-License-Identifier: MPL-2.0

//! Architecture dependent CPU-local information utilities.

/// Gets the base address for the CPU local storage.
///
/// On AArch64, TPIDR_EL1 stores the base address of CPU-local storage,
/// set by `bsp_boot.S`.
pub(crate) fn get_base() -> u64 {
    let base: u64;
    // SAFETY: TPIDR_EL1 holds the CPU-local base address set during boot.
    // Reading it via `mrs` is always safe and cannot be reordered by the
    // compiler (inline asm acts as a compiler barrier).
    unsafe { core::arch::asm!("mrs {0}, tpidr_el1", out(reg) base) };
    base
}
