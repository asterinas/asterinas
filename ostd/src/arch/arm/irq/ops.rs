// SPDX-License-Identifier: MPL-2.0

//! Interrupt operations.

use core::arch::asm;

// FIXME: Mark this as unsafe. See
// <https://github.com/asterinas/asterinas/issues/1120#issuecomment-2748696592>.
pub(crate) fn enable_local() {
    // SAFETY: The safety is upheld by the caller.
    unsafe { asm!("msr daifclr, 0b0010") };
}

/// Enables local IRQs and halts the CPU to wait for interrupts.
///
/// This method guarantees that no interrupts can occur in the middle. In other words, IRQs must
/// either have been processed before this method is called, or they must wake the CPU up from the
/// halting state.
//
// FIXME: Mark this as unsafe. See
// <https://github.com/asterinas/asterinas/issues/1120#issuecomment-2748696592>.
pub(crate) fn enable_local_and_halt() {
    // ARM(R) Architecture Reference Manual (ARMv8, for ARMv8-A architecture profile) says:
    // "The following are WFI wake-up events: [..] IRQ interrupt [..] regardless of the value of the
    // corresponding PSTATE.{A,I,F} mask bit."
    //
    // So we can use `wfi` even if IRQs are disabled. Pending IRQs can still wake up the CPU, but
    // they will only occur later when we enable local IRQs.
    //
    // SAFETY: It is safe to halt the CPU and wait for interrupts.
    unsafe { asm!("wfi") };

    enable_local();
}

pub(crate) fn disable_local() {
    // SAFEY: It is safe to disable local IRQs.
    unsafe { asm!("msr daifset, 0b0010") };
}

/// Disables local IRQs and halts the CPU forever.
pub(crate) fn disable_local_and_halt() -> ! {
    disable_local();
    loop {
        // SAFETY: It is safe to halt the CPU and wait for interrupts.
        unsafe { asm!("wfi") };
    }
}

// Process state (Pstate), IRQ mask (I) bit.
pub(in crate::arch) const PSTATE_I: usize = 1 << 7;

pub(crate) fn is_local_enabled() -> bool {
    let daif: usize;
    // SAFETY: It is safe to read the Interrupt Mask Bits (DAIF).
    unsafe { asm!("mrs {}, daif", out(reg) daif) };
    // Note that this differs from what should be written to the DAIFset and DAIFclr registers.
    daif & PSTATE_I == 0
}
