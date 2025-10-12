// SPDX-License-Identifier: MPL-2.0

//! Interrupt operations.

// FIXME: Mark this as unsafe. See
// <https://github.com/asterinas/asterinas/issues/1120#issuecomment-2748696592>.
pub(crate) fn enable_local() {
    // SAFETY: The safety is upheld by the caller.
    unsafe { riscv::interrupt::enable() }
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
    // RISC-V Instruction Set Manual, Machine-Level ISA, Version 1.13 says:
    // "The WFI instruction can also be executed when interrupts are disabled. The operation of WFI
    // must be unaffected by the global interrupt bits in `mstatus` (MIE and SIE) [..]"
    //
    // So we can use `wfi` even if IRQs are disabled. Pending IRQs can still wake up the CPU, but
    // they will only occur later when we enable local IRQs.
    riscv::asm::wfi();

    // SAFETY: The safety is upheld by the caller.
    unsafe { riscv::interrupt::enable() }
}

pub(crate) fn disable_local() {
    riscv::interrupt::disable();
}

pub(crate) fn is_local_enabled() -> bool {
    riscv::register::sstatus::read().sie()
}
