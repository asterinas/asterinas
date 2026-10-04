// SPDX-License-Identifier: MPL-2.0

pub(super) fn write_cr2_raw(value: u64) {
    // SAFETY: It is safe to write to the CR2 register.
    unsafe { core::arch::asm!("mov cr2, {}", in(reg) value) };
}

/// Reads the raw value of `CR3`.
pub(super) fn read_cr3_raw() -> usize {
    let value: usize;
    // SAFETY: It is safe to read the CR3 register.
    unsafe { core::arch::asm!("mov {}, cr3", out(reg) value) };
    value
}
