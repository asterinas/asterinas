// SPDX-License-Identifier: MPL-2.0

//! Handles trap.

#[expect(clippy::module_inception)]
mod trap;

use loongArch64::register::{
    estat::{self, Exception, Interrupt, Trap},
    prmd::Prmd,
};
pub(super) use trap::RawUserContext;
pub use trap::TrapFrame;

use crate::{
    arch::{
        cpu::context::CpuExceptionInfo,
        irq::{HwIrqLine, disable_local, enable_local},
        mm::tlb_flush_addr,
    },
    cpu::PrivilegeLevel,
    irq::call_irq_callback_functions,
};

/// Initializes trap handling on LoongArch.
///
/// This function will:
/// - Set `ecfg`'s VS bit to zero.
/// - Set `eentry` to the trap entry.
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

    let cause = estat::read().cause();
    let badi = loongArch64::register::badi::read().raw();
    let badv = loongArch64::register::badv::read().vaddr();
    let era = loongArch64::register::era::read().raw();

    // Page fault handling may wait for TLB shootdowns, so preserve the IRQ state
    // from before the exception.
    let was_irq_enabled = Prmd::from(f.prmd).pie();

    match cause {
        Trap::Exception(exception) => match exception {
            Exception::LoadPageFault
            | Exception::StorePageFault
            | Exception::PageModifyFault
            | Exception::FetchPageFault
            | Exception::PageNonReadableFault
            | Exception::PageNonExecutableFault
            | Exception::PagePrivilegeIllegal => {
                tlb_flush_addr(badv);
                crate::debug!(
                    "Page fault occurred in kernel: {exception:?}, badv: {badv:#x?}, badi: {badi:#x?}, era: {era:#x?}"
                );
                enable_local_if(was_irq_enabled);
                crate::mm::fault::handle_user_page_fault(
                    f,
                    &CpuExceptionInfo {
                        code: exception,
                        page_fault_addr: badv,
                        error_code: 0,
                    },
                    badv,
                );
                disable_local_if(was_irq_enabled);
            }
            Exception::FetchInstructionAddressError => todo!(),
            Exception::MemoryAccessAddressError => todo!(),
            Exception::AddressNotAligned => todo!(),
            Exception::BoundsCheckFault => todo!(),
            Exception::Syscall => todo!(),
            Exception::Breakpoint => todo!(),
            Exception::InstructionNotExist => todo!(),
            Exception::InstructionPrivilegeIllegal => todo!(),
            Exception::FloatingPointUnavailable => todo!(),
            Exception::TLBRFill => unreachable!(),
        },
        Trap::Interrupt(interrupt) => {
            match interrupt {
                Interrupt::SWI0 => todo!(),
                Interrupt::SWI1 => todo!(),
                Interrupt::HWI0
                | Interrupt::HWI1
                | Interrupt::HWI2
                | Interrupt::HWI3
                | Interrupt::HWI4
                | Interrupt::HWI5
                | Interrupt::HWI6
                | Interrupt::HWI7 => {
                    crate::debug!("Handling hardware interrupt: {:?}", interrupt);
                    while let Some(irq_num) = crate::arch::irq::chip::claim() {
                        // Call the IRQ callback functions for the claimed interrupt
                        call_irq_callback_functions(
                            f,
                            &HwIrqLine::new(irq_num),
                            PrivilegeLevel::Kernel,
                        );
                    }
                }
                Interrupt::PMI => todo!(),
                Interrupt::Timer => todo!(),
                Interrupt::IPI => todo!(),
            }
        }
        Trap::MachineError(machine_error) => panic!(
            "Machine error: {machine_error:?}, badv: {badv:#x?}, badi: {badi:#x?}, era: {era:#x?}"
        ),
        Trap::Unknown => panic!("Unknown trap, badv: {badv:#x?}, badi: {badi:#x?}, era: {era:#x?}"),
    }
}
