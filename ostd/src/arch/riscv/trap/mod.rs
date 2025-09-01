// SPDX-License-Identifier: MPL-2.0

//! Handles trap.

#[expect(clippy::module_inception)]
mod trap;

use core::sync::atomic::Ordering;

use riscv::{
    interrupt::supervisor::{Exception, Interrupt},
    register::scause::Trap,
};
use spin::Once;
pub use trap::TrapFrame;
pub(super) use trap::{RawUserContext, SSTATUS_FS_MASK, SSTATUS_SUM};

use crate::{
    arch::{
        cpu::context::CpuException,
        irq::{disable_local, enable_local, HwIrqLine, InterruptSource, IRQ_CHIP},
        timer::TIMER_IRQ_NUM,
    },
    cpu::{CpuId, PrivilegeLevel},
    ex_table::ExTable,
    irq::call_irq_callback_functions,
    mm::MAX_USERSPACE_VADDR,
};

/// Initializes interrupt handling on RISC-V.
///
/// # Safety
///
/// On the current CPU, this function must be called
/// - only once and
/// - before any trap can occur.
pub(crate) unsafe fn init_on_cpu() {
    // SAFETY: The caller ensures the safety conditions.
    unsafe {
        self::trap::init_on_cpu();
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
    let Ok(cause) = Trap::<Interrupt, Exception>::try_from(scause.cause()) else {
        panic!(
            "Cannot handle unknown trap, scause: {:#x}, trapframe: {:#x?}.",
            scause.bits(),
            f
        );
    };

    let exception = match cause {
        Trap::Interrupt(interrupt) => {
            handle_irq(f, interrupt, PrivilegeLevel::Kernel);
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
        CpuException::InstructionPageFault(fault_addr)
        | CpuException::LoadPageFault(fault_addr)
        | CpuException::StorePageFault(fault_addr) => {
            if (0..MAX_USERSPACE_VADDR).contains(&fault_addr) {
                handle_user_page_fault(f, &exception);
            } else {
                panic!("Cannot handle page fault in kernel space, exception: {:#x?}, trapframe: {:#x?}.", exception, f);
            }
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

pub(super) fn handle_irq(trap_frame: &TrapFrame, interrupt: Interrupt, priv_level: PrivilegeLevel) {
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
        Interrupt::SupervisorSoft => {
            let ipi_irq_num = super::irq::ipi::IPI_IRQ.get().unwrap().num();
            call_irq_callback_functions(
                trap_frame,
                &HwIrqLine::new(ipi_irq_num, InterruptSource::Software),
                priv_level,
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

fn handle_user_page_fault(f: &mut TrapFrame, exception: &CpuException) {
    let handler = USER_PAGE_FAULT_HANDLER
        .get()
        .expect("Page fault handler is missing");

    let res = handler(exception);
    // Copying bytes by bytes can recover directly
    // if handling the page fault successfully.
    if res.is_ok() {
        return;
    }

    // Use the exception table to recover to normal execution.
    if let Some(addr) = ExTable::find_recovery_inst_addr(f.sepc) {
        f.sepc = addr;
    } else {
        panic!(
            "Failed to handle page fault, exception: {:?}, trapframe: {:#x?}.",
            exception, f
        )
    }
}
