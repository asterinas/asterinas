// SPDX-License-Identifier: MPL-2.0

//! Handles trap.

#[expect(clippy::module_inception)]
mod trap;

use core::sync::atomic::Ordering;

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
    mm::MAX_USERSPACE_VADDR,
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

    use riscv::register::scause::Trap::*;

    let scause = riscv::register::scause::read();
    match scause.cause() {
        Interrupt(interrupt) => {
            use riscv::register::scause::Interrupt::*;

            match interrupt {
                SupervisorTimer => {
                    call_irq_callback_functions(
                        f,
                        &HwIrqLine::new(
                            TIMER_IRQ_NUM.load(Ordering::Relaxed),
                            InterruptSource::Timer,
                        ),
                        PrivilegeLevel::Kernel,
                    );
                }
                SupervisorExternal => {
                    // No races because we are in IRQs.
                    let current_cpu = CpuId::current_racy().as_usize() as u32;
                    while let Some(hw_irq_line) =
                        IRQ_CHIP.get().unwrap().claim_interrupt(current_cpu)
                    {
                        call_irq_callback_functions(f, &hw_irq_line, PrivilegeLevel::Kernel);
                    }
                }
                SupervisorSoft => todo!(),
                Unknown => {
                    panic!(
                        "Cannot handle unknown supervisor interrupt, scause: {:#x}, trapframe: {:#x?}.",
                        scause.bits(), f
                    );
                }
            }
        }
        Exception(e) => {
            use CpuException::*;

            let exception = e.into();
            // The IRQ state before trapping. We need to ensure that the IRQ state
            // during exception handling is consistent with the state before the trap.
            let was_irq_enabled = riscv::register::sstatus::read().spie();
            enable_local_if(was_irq_enabled);
            match exception {
                InstructionPageFault(fault_addr)
                | LoadPageFault(fault_addr)
                | StorePageFault(fault_addr) => {
                    if (0..MAX_USERSPACE_VADDR).contains(&fault_addr) {
                        handle_user_page_fault(f, &exception);
                    }
                }
                Unknown => {
                    panic!(
                        "Cannot handle unknown exception, scause: {:#x}, trapframe: {:#x?}.",
                        scause.bits(),
                        f
                    );
                }
                _ => {
                    panic!(
                        "Cannot handle exception, exception: {:#x?}, trapframe: {:#x?}.",
                        exception, f
                    );
                }
            };
            disable_local_if(was_irq_enabled);
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

fn handle_user_page_fault(f: &mut TrapFrame, exception: &CpuException) {
    let handler = USER_PAGE_FAULT_HANDLER
        .get()
        .expect("Page fault handler is missing");

    handler(exception).unwrap_or_else(|_| {
        panic!(
            "Failed to handle page fault, exception: {:?}, trapframe: {:#x?}.",
            exception, f
        )
    });
}
