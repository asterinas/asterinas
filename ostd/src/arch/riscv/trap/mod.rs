// SPDX-License-Identifier: MPL-2.0

//! Handles trap.

#[expect(clippy::module_inception)]
mod trap;

use core::sync::atomic::Ordering;

use riscv::register::scause::{Interrupt, Trap};
use spin::Once;
pub(super) use trap::RawUserContext;
pub use trap::TrapFrame;

use crate::{
    arch::{
        cpu::context::CpuException,
        irq::{disable_local, enable_local, HwIrqLine, InterruptSource, IRQ_CHIP},
        timer::TIMER_IRQ_NUM,
    },
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
    fn enable_local_if(cond: bool) {
        if cond {
            enable_local();
        }
    }

    fn disable_local_if(cond: bool) {
        if cond {
            disable_local();
        }
    }

    let scause = riscv::register::scause::read();
    let exception = match scause.cause() {
        Trap::Interrupt(interrupt) => {
            call_irq_callback_functions_by_scause(
                f,
                scause.bits(),
                interrupt,
                PrivilegeLevel::Kernel,
            );
            return;
        }
        Trap::Exception(raw_exception) => {
            let stval = riscv::register::stval::read();
            CpuException::new(raw_exception, stval)
        }
    };

    // The IRQ state before trapping. We need to ensure that the IRQ state
    // during exception handling is consistent with the state before the trap.
    const SSTATUS_SPIE: usize = 1 << 5;
    let was_irq_enabled = (f.sstatus & SSTATUS_SPIE) != 0;

    enable_local_if(was_irq_enabled);
    match exception {
        CpuException::Unknown => {
            panic!(
                "Cannot handle unknown exception, scause: {:#x}, trapframe: {:#x?}.",
                scause.bits(),
                f
            );
        }
        _ => {
            panic!(
                "Cannot handle kernel exception, exception: {:#x?}, trapframe: {:#x?}.",
                exception, f
            );
        }
    };
    disable_local_if(was_irq_enabled);
}

pub(super) fn call_irq_callback_functions_by_scause(
    trap_frame: &TrapFrame,
    scause: usize,
    interrupt: Interrupt,
    priv_level: PrivilegeLevel,
) {
    match interrupt {
        Interrupt::SupervisorTimer => {
            call_irq_callback_functions(
                trap_frame,
                &HwIrqLine::new(
                    TIMER_IRQ_NUM.load(Ordering::Relaxed),
                    InterruptSource::Timer,
                ),
                priv_level,
            );
        }
        Interrupt::SupervisorExternal => {
            // No races because we are in IRQs.
            let current_cpu = CpuId::current_racy().into();
            while let Some(hw_irq_line) = IRQ_CHIP.get().unwrap().claim_interrupt(current_cpu) {
                call_irq_callback_functions(trap_frame, &hw_irq_line, priv_level);
            }
        }
        Interrupt::SupervisorSoft => todo!(),
        Interrupt::Unknown => {
            panic!(
                "Cannot handle unknown supervisor interrupt, scause: {:#x}, trapframe: {:#x?}.",
                scause, trap_frame
            );
        }
    }
}

#[expect(clippy::type_complexity)]
static USER_PAGE_FAULT_HANDLER: Once<fn(&CpuException) -> core::result::Result<(), ()>> =
    Once::new();

/// Injects a custom handler for page faults that occur in the kernel and
/// are caused by user-space address.
pub fn inject_user_page_fault_handler(
    handler: fn(info: &CpuException) -> core::result::Result<(), ()>,
) {
    USER_PAGE_FAULT_HANDLER.call_once(|| handler);
}
