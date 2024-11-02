// SPDX-License-Identifier: MPL-2.0

//! Handles trap.

mod trap;

use riscv::register::scause::Interrupt;
pub use trap::{GeneralRegs, TrapFrame, UserContext};

use crate::{arch::timer::timer_callback, cpu_local_cell, trap::disable_local};

cpu_local_cell! {
    static IS_KERNEL_INTERRUPTED: bool = false;
}

/// Initialize interrupt handling on RISC-V.
pub unsafe fn init(on_bsp: bool) {
    self::trap::init();
}

/// Returns true if this function is called within the context of an IRQ handler
/// and the IRQ occurs while the CPU is executing in the kernel mode.
/// Otherwise, it returns false.
pub fn is_kernel_interrupted() -> bool {
    IS_KERNEL_INTERRUPTED.load()
}

/// Handle traps (only from kernel).
#[no_mangle]
extern "C" fn trap_handler(f: &mut TrapFrame) {
    use riscv::register::scause::Trap;

    match riscv::register::scause::read().cause() {
        Trap::Interrupt(interrupt) => {
            let _guard = disable_local();
            IS_KERNEL_INTERRUPTED.store(true);
            match interrupt {
                Interrupt::SupervisorSoft => todo!(),
                Interrupt::SupervisorTimer => timer_callback(),
                Interrupt::SupervisorExternal => todo!(),
                Interrupt::Unknown => todo!(),
            }
            IS_KERNEL_INTERRUPTED.store(false);
        }
        Trap::Exception(e) => {
            let stval = riscv::register::stval::read();
            panic!(
                "Cannot handle kernel cpu exception: {e:?}. stval: {stval:#x}, trapframe: {f:#x?}.",
            );
        }
    }
}
