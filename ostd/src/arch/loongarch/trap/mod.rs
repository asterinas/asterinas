// SPDX-License-Identifier: MPL-2.0

//! Handles trap.

mod trap;

use core::arch::asm;

use loongArch64::register::estat::{self, Exception, Interrupt, Trap};
use spin::Once;
pub(super) use trap::RawUserContext;
pub use trap::TrapFrame;

use crate::{
    arch::{boot::loongarch_boot, mm::tlb_flush_addr},
    cpu::context::CpuExceptionInfo,
    cpu_local_cell,
    mm::MAX_USERSPACE_VADDR,
    trap::call_irq_callback_functions,
};

cpu_local_cell! {
    static IS_KERNEL_INTERRUPTED: bool = false;
}

/// Initialize trap handling on LoongArch.
pub unsafe fn init() {
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
    let cause = estat::read().cause();
    let badi = loongArch64::register::badi::read().raw();
    let badv = loongArch64::register::badv::read().vaddr();
    let era = loongArch64::register::era::read().raw();

    match cause {
        Trap::Exception(exception) => match exception {
            Exception::LoadPageFault
            | Exception::StorePageFault
            | Exception::FetchPageFault
            | Exception::PageModifyFault
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
                        page_fault_addr: page_fault_addr,
                        error_code: 0,
                    })
                    .is_ok()
                {
                    return;
                }
                panic!("User page fault handler failed: addr: {page_fault_addr:#x}, err: {exception:?}");
            }
            Exception::PageModifyFault => {
                unimplemented!()
            }
            Exception::PageNonReadableFault => todo!(),
            Exception::PageNonExecutableFault => todo!(),
            Exception::PagePrivilegeIllegal => todo!(),
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
            IS_KERNEL_INTERRUPTED.store(true);
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
                    while let Some(irq) = crate::arch::kernel::irq::claim() {
                        // Call the IRQ callback functions for the claimed interrupt
                        call_irq_callback_functions(f, irq as _);
                    }
                }
                Interrupt::PMI => todo!(),
                Interrupt::Timer => todo!(),
                Interrupt::IPI => todo!(),
            }
            IS_KERNEL_INTERRUPTED.store(false);
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
