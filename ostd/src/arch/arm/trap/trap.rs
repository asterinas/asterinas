// SPDX-License-Identifier: MPL-2.0 OR MIT
//
// The original source code is from [trapframe-rs](https://github.com/rcore-os/trapframe-rs),
// which is released under the following license:
//
// SPDX-License-Identifier: MIT
//
// Copyright (c) 2020 - 2024 Runji Wang
//
// We make the following new changes:
// * Implement the `trap_handler` of Asterinas.
//
// These changes are released under the following license:
//
// SPDX-License-Identifier: MPL-2.0

use core::arch::{asm, global_asm};

use crate::{arch::cpu::context::GeneralRegs, irq::DisabledLocalIrqGuard, mm::fault::TrapFrameApi};

/// Saved registers on a trap.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::arch) struct RawUserContext {
    /// Trap num: Source and Kind
    pub(in crate::arch) trap_num: usize,
    /// Reserved for internal use
    pub(in crate::arch) __reserved: usize,
    /// Exception Link Register, elr_el1
    pub(in crate::arch) elr: usize,
    /// Saved Process Status Register, spsr_el1
    pub(in crate::arch) spsr: usize,
    /// Stack Pointer, sp_el0
    pub(in crate::arch) sp: usize,
    /// Software Thread ID Register, tpidr_el0
    pub(in crate::arch) tpidr: usize,
    /// General registers
    /// Must be last in this struct
    pub(in crate::arch) general: GeneralRegs,
}

global_asm!(include_str!("trap.S"));

/// Initializes interrupt handling on ARM.
///
/// This function will:
/// - Set `vbar_el1` to internal exception vector.
///
/// # Safety
///
/// On the current CPU, this function must be called
/// - only once and
/// - before any trap can occur.
pub(super) unsafe fn init_on_cpu() {
    // SAFETY: We believe that these assembly instructions correctly set up
    // the trap handling for the current CPU without side effects.
    unsafe {
        // Set the exception vector address.
        asm!("msr vbar_el1, {}", in(reg) __vectors as *const () as usize);
    }
}

/// Trap frame of kernel interrupt
///
/// # Trap handler
///
/// You need to define a handler function like this:
///
/// ```no_run
/// // SAFETY: The name does not collide with other symbols.
/// #[unsafe(no_mangle)]
/// pub extern "C" fn trap_handler(tf: &mut TrapFrame) {
///     println!("TRAP! tf: {:#x?}", tf);
/// }
/// ```
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct TrapFrame {
    /// Trap num: Source and Kind
    pub trap_num: usize,
    /// Reserved for internal use
    pub __reserved: usize,
    /// Exception Link Register, elr_el1
    pub elr: usize,
    /// Saved Process Status Register, spsr_el1
    pub spsr: usize,
    /// Stack Pointer, sp_el1
    pub sp: usize,
    /// Software Thread ID Register, tpidr_el1
    pub tpidr: usize,
    /// General registers
    /// Must be last in this struct
    pub general: GeneralRegs,
}

impl TrapFrameApi for TrapFrame {
    fn set_instruction_pointer(&mut self, ip: usize) {
        self.elr = ip;
    }

    fn instruction_pointer(&self) -> usize {
        self.elr
    }
}

impl RawUserContext {
    /// Goes to user space with the context, and comes back when a trap occurs.
    ///
    /// On return, the context will be reset to the status before the trap.
    /// Trap reason and error code will be placed at `trap_num`.
    pub(in crate::arch) fn run(&mut self, guard: DisabledLocalIrqGuard) {
        // Return to userspace with interrupts disabled. Otherwise, interrupts
        // after switching `sp` will mess up the CPU state.
        core::mem::forget(guard);

        unsafe { run_user(self) };
    }
}

unsafe extern "C" {
    unsafe fn __vectors();
    unsafe fn run_user(regs: &mut RawUserContext);
}
