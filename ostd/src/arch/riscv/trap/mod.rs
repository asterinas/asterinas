// SPDX-License-Identifier: MPL-2.0

//! Handles trap.

#[expect(clippy::module_inception)]
mod trap;

use riscv::{
    interrupt::supervisor::{Exception, Interrupt},
    register::scause::Trap,
};
pub use trap::TrapFrame;
pub(super) use trap::{RawUserContext, SSTATUS_FS_MASK, SSTATUS_SUM};

use crate::{
    arch::{
        cpu::context::CpuException,
        irq::{HwIrqLine, IRQ_CHIP, InterruptSource, disable_local, enable_local},
        timer::TIMER_IRQ,
    },
    cpu::PrivilegeLevel,
    irq::call_irq_callback_functions,
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
        trap::init_on_cpu();
    }
}

/// Handle traps (only from kernel).
// SAFETY: The name does not collide with other symbols.
#[unsafe(no_mangle)]
unsafe extern "C" fn trap_handler(f: &mut TrapFrame) {
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
            "Cannot handle unknown trap: {:#x?}; trapframe: {:#x?}",
            scause, f
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
            crate::mm::fault::handle_user_page_fault(f, &exception, fault_addr);
        }
        _ => {
            panic!(
                "Cannot handle kernel CPU exception: {:#x?}; trapframe: {:#x?}",
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
                &HwIrqLine::new(TIMER_IRQ.get().unwrap().num(), InterruptSource::Timer),
                priv_level,
            );
        }
        Interrupt::SupervisorExternal => {
            // No races because we're in IRQs.
            let hart_id = crate::arch::boot::smp::get_current_hart_id();
            while let Some(hw_irq_line) = IRQ_CHIP.get().unwrap().claim_interrupt(hart_id) {
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
