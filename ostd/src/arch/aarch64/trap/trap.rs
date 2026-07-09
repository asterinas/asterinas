// SPDX-License-Identifier: MPL-2.0

//! Low-level trap handling: the exception vector table and register frames.

use core::arch::global_asm;

use crate::arch::cpu::context::GeneralRegs;

global_asm!(include_str!("trap.S"));

/// Installs the exception vector table for the current CPU.
///
/// # Safety
///
/// On the current CPU, this function must be called
/// - only once, and
/// - before any trap can occur.
pub(super) unsafe fn init_on_cpu() {
    unsafe extern "C" {
        fn exception_vector_table();
    }
    // SAFETY: The symbol refers to a correctly aligned 16-entry vector table.
    unsafe {
        core::arch::asm!(
            "msr vbar_el1, {vbar}",
            "isb",
            vbar = in(reg) exception_vector_table as *const () as usize,
            options(nostack, preserves_flags),
        );
    }
}

/// The saved register state on a kernel trap.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TrapFrame {
    /// General registers (`x0`-`x30`, `sp`).
    pub general: GeneralRegs,
    /// Exception link register (the interrupted PC).
    pub elr: usize,
    /// Saved program status register.
    pub spsr: usize,
    /// Exception syndrome register.
    pub esr: usize,
}

/// The saved register state used to run and return from userspace.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::arch) struct RawUserContext {
    /// General registers (`x0`-`x30`, `sp`/`SP_EL0`).
    pub(in crate::arch) general: GeneralRegs,
    /// Exception link register (user PC).
    pub(in crate::arch) elr: usize,
    /// Saved program status register.
    pub(in crate::arch) spsr: usize,
    /// Thread pointer (`TPIDR_EL0`).
    pub(in crate::arch) tpidr: usize,
    /// Exception syndrome captured on the last return to the kernel.
    pub(in crate::arch) esr: usize,
    /// Fault address captured on the last return to the kernel.
    pub(in crate::arch) far: usize,
    /// The kind of trap that returned control to the kernel: 0 = synchronous
    /// (syscall/exception), 1 = IRQ/FIQ. Written by the user exception vectors.
    pub(in crate::arch) trap_kind: usize,
}

/// `trap_kind` value for a synchronous exception (syscall/fault) from userspace.
pub(in crate::arch) const TRAP_KIND_SYNC: usize = 0;
/// `trap_kind` value for an IRQ/FIQ taken from userspace.
pub(in crate::arch) const TRAP_KIND_IRQ: usize = 1;

impl RawUserContext {
    /// Enters userspace with this context, returning when a trap occurs.
    pub(in crate::arch) fn run(&mut self) {
        let guard = crate::irq::disable_local();
        crate::task::call_pre_user_run_handler(&guard);
        // Return to userspace with interrupts disabled; they are re-enabled by
        // the trap handler after switching back to the kernel.
        core::mem::forget(guard);

        // SAFETY: `self` is a valid user context; `run_user` restores it, enters
        // EL0, and writes the trap state back on return.
        unsafe { run_user(self) };
    }
}

unsafe extern "C" {
    unsafe fn run_user(regs: &mut RawUserContext);
}
