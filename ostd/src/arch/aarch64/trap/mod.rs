// SPDX-License-Identifier: MPL-2.0

//! Handles traps.

#[expect(clippy::module_inception)]
mod trap;

use spin::Once;
pub use trap::TrapFrame;
pub(in crate::arch) use trap::{RawUserContext, TRAP_KIND_IRQ};

use crate::{
    arch::cpu::context::CpuException, cpu::PrivilegeLevel, ex_table::ExTable,
    mm::MAX_USERSPACE_VADDR,
};

/// Initializes interrupt handling on the current CPU.
///
/// # Safety
///
/// On the current CPU, this function must be called
/// - only once, and
/// - before any trap can occur.
pub(crate) unsafe fn init_on_cpu() {
    // SAFETY: The caller ensures the safety conditions.
    unsafe { trap::init_on_cpu() };
}

/// Handles a synchronous exception (or SError) taken from the kernel.
// SAFETY: The name does not collide with other symbols.
#[unsafe(no_mangle)]
unsafe extern "C" fn trap_handler(f: &mut TrapFrame) {
    let exception = CpuException::new(f.esr, read_far());

    match exception {
        CpuException::InstructionAbort(info) | CpuException::DataAbort(info)
            if info.is_page_fault() =>
        {
            let fault_addr = info.far;
            if (0..MAX_USERSPACE_VADDR).contains(&fault_addr) {
                handle_user_page_fault(f, &exception);
            } else {
                panic!(
                    "Cannot handle kernel page fault, exception: {:#x?}, trapframe: {:#x?}.",
                    exception, f
                );
            }
        }
        _ => {
            panic!(
                "Cannot handle kernel exception, exception: {:#x?}, trapframe: {:#x?}.",
                exception, f
            );
        }
    }
}

/// Handles an IRQ/FIQ taken from the kernel.
// SAFETY: The name does not collide with other symbols.
#[unsafe(no_mangle)]
unsafe extern "C" fn irq_handler(f: &mut TrapFrame) {
    super::irq::handle_irq(f, PrivilegeLevel::Kernel);
}

fn read_far() -> usize {
    let far;
    // SAFETY: Reading `FAR_EL1` has no side effects.
    unsafe { core::arch::asm!("mrs {}, far_el1", out(reg) far, options(nostack, nomem)) };
    far
}

#[expect(clippy::type_complexity)]
static USER_PAGE_FAULT_HANDLER: Once<fn(&CpuException) -> Result<(), ()>> = Once::new();

/// Injects a custom handler for page faults that occur in the kernel and are
/// caused by a user-space address.
pub fn inject_user_page_fault_handler(handler: fn(info: &CpuException) -> Result<(), ()>) {
    USER_PAGE_FAULT_HANDLER.call_once(|| handler);
}

fn handle_user_page_fault(f: &mut TrapFrame, exception: &CpuException) {
    let handler = USER_PAGE_FAULT_HANDLER
        .get()
        .expect("Page fault handler is missing");

    if handler(exception).is_ok() {
        return;
    }

    // Recover through the exception table if possible.
    if let Some(addr) = ExTable::find_recovery_inst_addr(f.elr) {
        f.elr = addr;
    } else {
        panic!(
            "Failed to handle page fault, exception: {:?}, trapframe: {:#x?}.",
            exception, f
        )
    }
}
