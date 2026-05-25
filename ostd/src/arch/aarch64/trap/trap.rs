// SPDX-License-Identifier: MPL-2.0

//! Trap frame and user context definitions.

use core::arch::global_asm;

use crate::arch::cpu::context::GeneralRegs;

global_asm!(include_str!("trap.S"));

crate::cpu_local_cell! {
    /// Points to the current CPU's active RawUserContext, or 0 if none.
    pub(super) static CURRENT_USER_CTX: usize = 0;
}

/// Trap frame of kernel interrupt.
/// Layout matches assembly in trap.S:
///   +0x000-0x0F8: GeneralRegs (x0..x30 + sp) = 31 regs * 8 bytes
///   +0x100:       spsr_el1
///   +0x108:       elr_el1
///
/// For EL0 traps, general.sp holds the user stack pointer (from SP_EL0).
/// For EL1 traps, general.sp holds the pre-exception kernel stack pointer.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct TrapFrame {
    /// General registers
    pub general: GeneralRegs,
    /// Saved Program Status Register
    pub spsr_el1: usize,
    /// Exception Link Register
    pub elr_el1: usize,
}

/// Saved registers on a trap.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::arch) struct RawUserContext {
    /// General registers
    pub(in crate::arch) general: GeneralRegs,
    /// Saved Program Status Register
    pub(in crate::arch) spsr_el1: usize,
    /// Exception Link Register
    pub(in crate::arch) elr_el1: usize,
    /// Software-only thread-local storage pointer.
    ///
    /// This is NOT saved/restored on exception entry/exit. It is used only by
    /// `clone()`/`exec()` to set the initial TLS value and by
    /// `activate_tls_pointer()` to write the hardware TPIDR_EL0 register at
    /// task entry. Context switches use `ThreadTls` instead — see
    /// `kernel/src/process/posix_thread/thread_local.rs`.
    pub(in crate::arch) tls_pointer: usize,
}

impl RawUserContext {
    /// Goes to user space with the context, and comes back when a trap occurs.
    ///
    /// On return, the context will contain the saved registers from the trap.
    pub(in crate::arch) fn run(&mut self) {
        crate::arch::irq::disable_local();
        CURRENT_USER_CTX.store(self as *mut RawUserContext as usize);
        // SAFETY: The assembly function saves callee-saved regs, sets up
        // ELR_EL1/SPSR_EL1, restores user regs, and executes eret.
        unsafe { run_user(self) };
        CURRENT_USER_CTX.store(0);
    }
}

// SAFETY: The assembly symbol names match `trap.S`.
unsafe extern "C" {
    fn trap_vectors();
    fn run_user(ctx: &mut RawUserContext);
    pub(super) fn run_user_done();
}

/// Initialize trap handling on the current CPU.
///
/// # Safety
///
/// On the current CPU, this function must be called
/// - only once and
/// - before any trap can occur.
pub(super) unsafe fn init_on_cpu() {
    // SAFETY: trap_vectors is a 2KB-aligned exception vector table.
    // Writing its address to VBAR_EL1 installs proper trap handling.
    unsafe {
        core::arch::asm!(
            "msr vbar_el1, {0}",
            in(reg) trap_vectors as *const () as usize,
        );
    }
}
