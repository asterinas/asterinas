// SPDX-License-Identifier: MPL-2.0

//! CPU context & state control and CPU local memory.

pub mod context;
pub mod local;

/// Halts the CPU.
///
/// This function halts the CPU until the next interrupt is received. By
/// halting, the CPU will enter the C-0 state and consume less power.
///
/// Since the function sleeps the CPU, it should not be used within an atomic
/// mode ([`crate::task::atomic_mode`]).
#[track_caller]
pub fn sleep_for_interrupt() {
    crate::task::atomic_mode::might_sleep();
    x86_64::instructions::hlt();
}

/// Writes the `PKRU` register.
pub fn write_pkru(pkru: u32) {
    // SAFETY: Writing to PKRU is safe since the kernel memory access is not
    // related to the protection keys for the user-mode pages.
    unsafe {
        core::arch::asm! {
            "wrpkru",
            in("eax") pkru,
            in("ecx") 0,
            in("edx") 0,
            options(nostack, preserves_flags)
        }
    }
}

/// Reads the `PKRU` register.
pub fn read_pkru() -> u32 {
    let mut pkru: u32;
    // SAFETY: Reading from PKRU is safe since the kernel memory access is not
    // related to the protection keys for the user-mode pages.
    unsafe {
        core::arch::asm! {
            "rdpkru",
            in("ecx") 0,
            out("eax") pkru,
            out("edx") _,
            options(nostack, preserves_flags)
        }
    }
    pkru
}
