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

use crate::arch::cpu::{
    context::GeneralRegs,
    extension::{IsaExtensions, has_extensions},
};

#[cfg(target_arch = "riscv32")]
global_asm!(
    r"
    .equ XLENB, 4
    .macro LOAD_SP a1, a2
        lw \a1, \a2*XLENB(sp)
    .endm
    .macro STORE_SP a1, a2
        sw \a1, \a2*XLENB(sp)
    .endm
"
);
#[cfg(target_arch = "riscv64")]
global_asm!(
    r"
    .equ XLENB, 8
    .macro LOAD_SP a1, a2
        ld \a1, \a2*XLENB(sp)
    .endm
    .macro STORE_SP a1, a2
        sd \a1, \a2*XLENB(sp)
    .endm
"
);

/// FPU status bits.
/// Reference: <https://riscv.github.io/riscv-isa-manual/snapshot/privileged/#sstatus>.
pub(in crate::arch) const SSTATUS_FS_MASK: usize = 0b11 << 13;
/// Supervisor User Memory access bit.
/// Reference: <https://riscv.github.io/riscv-isa-manual/snapshot/privileged/#sstatus>.
pub(in crate::arch) const SSTATUS_SUM: usize = 0b1 << 18;

global_asm!(include_str!("trap.S"), SSTATUS_FS_MASK = const SSTATUS_FS_MASK, SSTATUS_SUM = const SSTATUS_SUM);

/// Initialize interrupt handling for the current HART.
///
/// This function will:
/// - Set `sscratch` to 0.
/// - Set `stvec` to internal exception vector.
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
        // Set sscratch register to 0, indicating to exception vector that we
        // are presently executing in the kernel.
        asm!("csrw sscratch, zero");
        // Set the exception vector address.
        asm!("csrw stvec, {}", in(reg) trap_entry as usize);
    }
}

/// Trap frame of kernel interrupt
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
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct TrapFrame {
    /// General registers
    pub general: GeneralRegs,
    /// Supervisor Status
    pub sstatus: usize,
    /// Supervisor Exception Program Counter
    pub sepc: usize,
}

/// Saved registers on a trap.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub(in crate::arch) struct RawUserContext {
    /// General registers
    pub(in crate::arch) general: GeneralRegs,
    /// Supervisor Status
    pub(in crate::arch) sstatus: usize,
    /// Supervisor Exception Program Counter
    pub(in crate::arch) sepc: usize,
}

impl Default for RawUserContext {
    fn default() -> Self {
        let sstatus = if has_extensions(IsaExtensions::F)
            || has_extensions(IsaExtensions::D)
            || has_extensions(IsaExtensions::Q)
        {
            const SSTATUS_FS_INITIAL: usize = 0b01 << 13;
            SSTATUS_FS_INITIAL
        } else {
            0
        };

        Self {
            general: GeneralRegs::default(),
            sstatus,
            sepc: 0,
        }
    }
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

unsafe extern "C" {
    unsafe fn trap_entry();
    unsafe fn run_user(regs: &mut RawUserContext);
}
