// SPDX-License-Identifier: MPL-2.0

//! Handles trap.

#[expect(clippy::module_inception)]
mod trap;

use loongArch64::register::estat::{self, Exception, Interrupt, Trap};
use spin::Once;
pub(super) use trap::RawUserContext;
pub use trap::TrapFrame;

use crate::{
    arch::{cpu::context::CpuExceptionInfo, irq::HwIrqLine, mm::tlb_flush_addr},
    cpu::PrivilegeLevel,
    irq::call_irq_callback_functions,
    mm::MAX_USERSPACE_VADDR,
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
        self::trap::init_on_cpu();
    }
}

/// Handle traps (only from kernel).
#[no_mangle]
extern "C" fn trap_handler(f: &mut TrapFrame) {
    let cause = estat::read().cause();
    let badi = loongArch64::register::badi::read().raw();
    let badv = loongArch64::register::badv::read().vaddr();
    let era = loongArch64::register::era::read().raw();

    match cause {
        Trap::Exception(exception) => match exception {
            Exception::LoadPageFault
            | Exception::StorePageFault
            | Exception::FetchPageFault
            | Exception::PageNonReadableFault
            | Exception::PageNonExecutableFault
            | Exception::PagePrivilegeIllegal => {
                tlb_flush_addr(badv);
                log::debug!(
                    "Page fault occurred in kernel: {exception:?}, badv: {badv:#x?}, badi: {badi:#x?}, era: {era:#x?}"
                );
                let page_fault_addr = badv;
                // Check if the page fault is caused by user-space address
                if let Some(handler) = USER_PAGE_FAULT_HANDLER.get()
                    && (0..MAX_USERSPACE_VADDR).contains(&(page_fault_addr as usize))
                    && handler(&CpuExceptionInfo {
                        code: exception,
                        page_fault_addr,
                        error_code: 0,
                    })
                    .is_ok()
                {
                    return;
                }
                panic!(
                    "User page fault handler failed: addr: {page_fault_addr:#x}, err: {exception:?}"
                );
            }
            Exception::PageModifyFault => {
                unimplemented!()
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
                    log::debug!("Handling hardware interrupt: {:?}", interrupt);
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
