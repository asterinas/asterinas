// SPDX-License-Identifier: MPL-2.0

use core::arch::{asm, global_asm};

use crate::arch::cpu::context::GeneralRegs;

global_asm!(include_str!("trap.S"));

/// Initializes exception and interrupt handling for the current core.
///
/// # Safety
///
/// This function will:
/// - Set `eentry` to `trap_entry`
///
/// You **MUST NOT** modify these registers later.
pub unsafe fn init() {
    // When VS=0, the entry address for all exceptions and interrupts is the same
    loongArch64::register::ecfg::set_vs(0);
    // Configure the entry address for normal exceptions and interrupts
    loongArch64::register::eentry::set_eentry(trap_entry as usize);
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
#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub struct TrapFrame {
    /// General registers
    pub general: GeneralRegs,
    /// Pre-exception Mode Information
    pub prmd: usize,
    /// Exception Return Address
    pub era: usize,
    /// Extended Unit Enable
    pub euen: usize,
}

/// Saved registers on a trap.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub(in crate::arch) struct RawUserContext {
    /// General registers
    pub(in crate::arch) general: GeneralRegs,
    /// Pre-exception Mode Information
    pub(in crate::arch) prmd: usize,
    /// Exception Return Address
    pub(in crate::arch) era: usize,
    /// Extended Unit Enable
    pub(in crate::arch) euen: usize,
}

impl Default for RawUserContext {
    fn default() -> Self {
        Self {
            general: GeneralRegs::default(),
            prmd: 0b111, // User mode, enable interrupt
            era: 0,
            euen: 0,
        }
    }
}

impl RawUserContext {
    /// Goes to user space with the context, and comes back when a trap occurs.
    ///
    /// On return, the context will be reset to the status before the trap.
    /// Trap reason will be placed at `estat`.
    pub(in crate::arch) fn run(&mut self) {
        // Return to userspace with interrupts disabled. Otherwise, interrupts
        // after switching `SAVE_SCRATCH` will mess up the CPU state.
        crate::arch::irq::disable_local();
        unsafe { run_user(self as *mut RawUserContext) }
    }
}

unsafe extern "C" {
    unsafe fn trap_entry();
    unsafe fn run_user(regs: *mut RawUserContext);
}
