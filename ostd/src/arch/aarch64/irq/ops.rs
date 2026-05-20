// SPDX-License-Identifier: MPL-2.0

//! IRQ enable/disable operations via the DAIF register.

/// Enables local IRQs by clearing the IRQ mask in DAIF.
pub(crate) fn enable_local() {
    // SAFETY: DAIF writes are always safe.
    unsafe { core::arch::asm!("msr daifclr, #2") };
}

/// Disables local IRQs by setting the IRQ mask in DAIF.
pub(crate) fn disable_local() {
    // SAFETY: DAIF writes are always safe.
    unsafe { core::arch::asm!("msr daifset, #2") };
}

/// Disables local IRQs and halts the CPU forever.
pub(crate) fn disable_local_and_halt() -> ! {
    unsafe { core::arch::asm!("msr daifset, #2") };
    loop {
        // SAFETY: WFI is a privileged instruction that never faults.
        unsafe { core::arch::asm!("wfi") };
    }
}

/// Enables local IRQs and halts the CPU.
pub(crate) fn enable_local_and_halt() {
    unsafe { core::arch::asm!("msr daifclr, #2; wfi") };
}

/// Returns whether local IRQs are enabled (PSTATE.I, bit 7 of DAIF, is clear).
pub(crate) fn is_local_enabled() -> bool {
    let daif: u64;
    // SAFETY: Reading DAIF is always safe.
    unsafe { core::arch::asm!("mrs {0}, daif", out(reg) daif) };
    daif & (1 << 7) == 0
}
