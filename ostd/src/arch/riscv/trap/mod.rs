// SPDX-License-Identifier: MPL-2.0

//! Handles trap.

#[expect(clippy::module_inception)]
mod trap;

use core::sync::atomic::Ordering;

use riscv::register::scause::Interrupt;
use spin::Once;
pub(super) use trap::RawUserContext;
pub use trap::TrapFrame;

use super::{cpu::context::CpuExceptionInfo, timer::TIMER_IRQ_NUM};
use crate::{
    arch::irq::{HwIrqLine, InterruptSource, IRQ_CHIP},
    cpu::{CpuId, PrivilegeLevel},
    irq::call_irq_callback_functions,
};

/// Initializes interrupt handling on RISC-V.
pub(crate) unsafe fn init() {
    unsafe {
        self::trap::init();
    }
}

/// Handle traps (only from kernel).
#[no_mangle]
extern "C" fn trap_handler(f: &mut TrapFrame) {
    use riscv::register::scause::Trap;

    match riscv::register::scause::read().cause() {
        Trap::Interrupt(interrupt) => match interrupt {
            Interrupt::SupervisorTimer => {
                call_irq_callback_functions(
                    f,
                    &HwIrqLine::new(
                        TIMER_IRQ_NUM.load(Ordering::Relaxed),
                        InterruptSource::Timer,
                    ),
                    PrivilegeLevel::Kernel,
                );
            }
            Interrupt::SupervisorExternal => {
                // No races because we are in IRQs.
                let current_cpu = CpuId::current_racy().as_usize() as u32;
                while let Some(hw_irq_line) = IRQ_CHIP.get().unwrap().claim_interrupt(current_cpu) {
                    call_irq_callback_functions(f, &hw_irq_line, PrivilegeLevel::Kernel);
                }
            }
            Interrupt::SupervisorSoft => todo!(),
            _ => {
                panic!(
                        "cannot handle unknown supervisor interrupt: {interrupt:?}. trapframe: {f:#x?}.",
                    );
            }
        },
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
