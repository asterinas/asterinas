// SPDX-License-Identifier: MPL-2.0

//! Handles trap.

mod trap;

use riscv::register::scause::{Exception, Interrupt, Trap};
pub use trap::{GeneralRegs, TrapFrame, UserContext};

use super::ex_table::ExTable;
use crate::{
    arch::irq::TIMER_IRQ_LINE, cpu::CpuExceptionInfo, cpu_local_cell, mm::MAX_USERSPACE_VADDR,
    task::Task, trap::call_irq_callback_functions,
};

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

pub fn handle_external_interrupts(f: &TrapFrame) {
    while let Some(irq) = super::device::plic::claim_interrupt() {
        call_irq_callback_functions(f, irq.get() as usize);
    }
}

/// Handle traps (only from kernel).
#[no_mangle]
extern "C" fn trap_handler(f: &mut TrapFrame) {
    match riscv::register::scause::read().cause() {
        Trap::Interrupt(interrupt) => {
            IS_KERNEL_INTERRUPTED.store(true);
            match interrupt {
                Interrupt::SupervisorTimer => call_irq_callback_functions(f, TIMER_IRQ_LINE),
                Interrupt::SupervisorExternal => handle_external_interrupts(f),
                _ => todo!(),
            }
            IS_KERNEL_INTERRUPTED.store(false);
        }
        Trap::Exception(e) => {
            let stval = riscv::register::stval::read();
            // The actual user space implementation should be responsible
            // for providing mechanism to treat the 0 virtual address.
            if (0..MAX_USERSPACE_VADDR).contains(&stval) {
                handle_user_page_fault(f, stval, e);
            } else {
                panic!(
                    "Cannot handle kernel cpu exception: {e:?}. stval: {stval:#x}, trapframe: {f:#x?}.",
                );
            }
        }
    }
}

/// Handles page fault from user space.
fn handle_user_page_fault(f: &mut TrapFrame, page_fault_addr: usize, e: Exception) {
    let current_task = Task::current().unwrap();
    let user_space = current_task
        .user_space()
        .expect("the user space is missing when a page fault from the user happens.");

    let info = CpuExceptionInfo {
        code: e,
        page_fault_addr,
        error_code: 0,
    };

    let res = user_space.vm_space().handle_page_fault(&info);
    // Copying bytes by bytes can recover directly
    // if handling the page fault successfully.
    if res.is_ok() {
        return;
    }

    // Use the exception table to recover to normal execution.
    if let Some(addr) = ExTable::find_recovery_inst_addr(f.sepc) {
        f.sepc = addr;
    } else {
        panic!("Cannot handle user page fault; Trapframe: {:#x?}.", f);
    }
}
