// SPDX-License-Identifier: MPL-2.0

//! Interrupt operations.

use x86_64::registers::rflags::{self, RFlags};

// FIXME: Mark this as unsafe. See
// <https://github.com/asterinas/asterinas/issues/1120#issuecomment-2748696592>.
pub(crate) fn enable_local() {
    x86_64::instructions::interrupts::enable();
    // When emulated with QEMU, interrupts may not be delivered if a STI instruction is immediately
    // followed by a RET instruction. It is a BUG of QEMU, see the following patch for details.
    // https://lore.kernel.org/qemu-devel/20231210190147.129734-2-lrh2000@pku.edu.cn/
    x86_64::instructions::nop();
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
    // SAFETY:
    // 1. `sti` is safe to use because its safety requirement is upheld by the caller.
    // 2. `hlt` is safe to use because it halts the CPU for interrupts.
    unsafe {
        // Intel(R) 64 and IA-32 Architectures Software Developer's Manual says:
        // "If IF = 0, maskable hardware interrupts remain inhibited on the instruction boundary
        // following an execution of STI."
        //
        // So interrupts will only occur at or after the HLT instruction, which guarantee that
        // interrupts won't occur between enabling the local IRQs and halting the CPU.
        core::arch::asm!("sti", "hlt", options(nomem, nostack, preserves_flags),)
    };
}

pub(crate) fn disable_local() {
    x86_64::instructions::interrupts::disable();
}

pub(crate) fn is_local_enabled() -> bool {
    (rflags::read_raw() & RFlags::INTERRUPT_FLAG.bits()) != 0
}
