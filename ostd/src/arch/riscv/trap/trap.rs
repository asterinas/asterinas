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

#![allow(unfulfilled_lint_expectations)]

use core::arch::global_asm;

use riscv::register::stvec::TrapMode;

use crate::arch::cpu::context::GeneralRegs;

global_asm!(include_str!("trap.S"));

/// Initialize interrupt handling for the current HART.
///
/// # Safety
///
/// This function will:
/// - Set `sscratch` to 0.
/// - Set `stvec` to internal exception vector.
///
/// You **MUST NOT** modify these registers later.
pub unsafe fn init() {
    unsafe {
        // Set sscratch register to 0, indicating to exception vector that we are
        // presently executing in the kernel
        riscv::register::sscratch::write(0);
        // Set the exception vector address
        riscv::register::stvec::write(trap_entry as usize, TrapMode::Direct);
    }
}

/// Trap frame of kernel interrupt.
///
/// # Trap handler
///
/// You need to define a handler function like this:
///
/// ```no_run
/// #[no_mangle]
/// pub extern "C" fn trap_handler(tf: &mut TrapFrame) {
///     println!("TRAP! tf: {:#x?}", tf);
/// }
/// ```
#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub struct TrapFrame {
    /// General-purpose registers
    pub general_regs: GeneralRegs,
    /// Supervisor Status
    pub sstatus: usize,
    /// Supervisor Exception Program Counter
    pub sepc: usize,
}

/// Userspace context.
#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub(in crate::arch) struct RawUserContext {
    /// General-purpose registers
    pub(in crate::arch) general_regs: GeneralRegs,
    /// Supervisor Status
    pub(in crate::arch) sstatus: usize,
    /// Supervisor Exception Program Counter
    pub(in crate::arch) sepc: usize,
    /// Kernel stack pointer
    kernel_sp: usize,
}

impl RawUserContext {
    /// Goes to user space with the context, and comes back when a trap occurs.
    ///
    /// On return, the context will be reset to the status before the trap.
    /// Trap reason and error code will be placed at `scause` and `stval`.
    pub(in crate::arch) fn run(&mut self) {
        // Return to userspace with interrupts disabled. Otherwise, interrupts
        // after switching `sscratch` will mess up the CPU state.
        crate::arch::irq::disable_local();
        unsafe { run_user(self) }
    }
}

extern "C" {
    fn trap_entry();
    fn run_user(regs: &mut RawUserContext);
}
