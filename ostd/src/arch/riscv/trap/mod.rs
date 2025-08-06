// SPDX-License-Identifier: MPL-2.0

//! Handles trap.

mod trap;

use alloc::format;

use spin::Once;
pub(super) use trap::RawUserContext;
pub use trap::TrapFrame;

use crate::{
    arch::plic::claim_interrupt,
    cpu::{context::CpuException, CpuId},
    cpu_local_cell,
    mm::MAX_USERSPACE_VADDR,
    trap::call_irq_callback_functions,
};

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
    use riscv::register::scause::Trap::*;

    let scause = riscv::register::scause::read();
    match scause.cause() {
        Interrupt(interrupt) => {
            use riscv::register::scause::Interrupt::*;

            IS_KERNEL_INTERRUPTED.store(true);
            match interrupt {
                SupervisorTimer => {
                    crate::arch::timer::handle_timer_interrupt();
                }
                SupervisorExternal => {
                    while let irq_num = claim_interrupt(CpuId::current_racy().as_usize())
                        && irq_num != 0
                    {
                        call_irq_callback_functions(f, irq_num);
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
            IS_KERNEL_INTERRUPTED.store(false);
        }
        Exception(e) => {
            use CpuException::*;

            let exception = e.into();
            match exception {
                InstructionPageFault(fault_addr)
                | LoadPageFault(fault_addr)
                | StorePageFault(fault_addr) => {
                    if (0..MAX_USERSPACE_VADDR).contains(&fault_addr.0) {
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
                        "Cannot handle kernel exception, exception: {:?}, trapframe: {:#x?}.",
                        exception, f
                    );
                }
            };
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

    handler(exception).expect(&format!(
        "Failed to handle page fault, exception: {:?}, trapframe: {:#x?}.",
        exception, f
    ));
}
