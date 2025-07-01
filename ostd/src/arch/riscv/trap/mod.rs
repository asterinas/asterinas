// SPDX-License-Identifier: MPL-2.0

//! Handles trap.

mod trap;

use riscv::register::scause::Interrupt;
use spin::Once;
pub(super) use trap::RawUserContext;
pub use trap::TrapFrame;

use super::cpu::context::CpuExceptionInfo;
use crate::cpu_local_cell;

cpu_local_cell! {
    static IS_KERNEL_INTERRUPTED: bool = false;
}

/// Initializes interrupt handling on RISC-V.
pub(crate) unsafe fn init() {
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
            IS_KERNEL_INTERRUPTED.store(true);
            match interrupt {
                Interrupt::SupervisorTimer => {
                    crate::arch::timer::handle_timer_interrupt();
                }
                Interrupt::SupervisorExternal => todo!(),
                Interrupt::SupervisorSoft => todo!(),
                _ => {
                    panic!(
                        "cannot handle unknown supervisor interrupt: {interrupt:?}. trapframe: {f:#x?}.",
                    );
                }
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

#[expect(clippy::type_complexity)]
static USER_PAGE_FAULT_HANDLER: Once<fn(&CpuExceptionInfo) -> core::result::Result<(), ()>> =
    Once::new();

/// Injects a custom handler for page faults that occur in the kernel and
/// are caused by user-space address.
pub fn inject_user_page_fault_handler(
    handler: fn(info: &CpuExceptionInfo) -> core::result::Result<(), ()>,
) {
    USER_PAGE_FAULT_HANDLER.call_once(|| handler);
}
