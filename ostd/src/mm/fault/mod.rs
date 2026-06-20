// SPDX-License-Identifier: MPL-2.0

//! Page fault handling.

mod ex_table;

use spin::Once;

#[cfg(not(target_arch = "loongarch64"))]
use crate::arch::cpu::context::CpuException;
#[cfg(target_arch = "loongarch64")]
use crate::arch::cpu::context::CpuExceptionInfo as CpuException;
use crate::{
    arch::trap::TrapFrame,
    mm::{MAX_USERSPACE_VADDR, Vaddr, fault::ex_table::ExTable},
};

#[expect(clippy::type_complexity)]
static USER_PAGE_FAULT_HANDLER: Once<fn(&CpuException) -> Result<(), ()>> = Once::new();

/// Injects a custom handler for page faults that occur in the kernel and
/// are caused by user-space address.
pub fn inject_user_page_fault_handler(handler: fn(info: &CpuException) -> Result<(), ()>) {
    USER_PAGE_FAULT_HANDLER.call_once(|| handler);
}

/// The common interface that every CPU architecture-specific [`TrapFrame`] implements.
pub(crate) trait TrapFrameApi {
    /// Sets the instruction pointer.
    fn set_instruction_pointer(&mut self, ip: usize);

    /// Gets the instruction pointer.
    fn instruction_pointer(&self) -> usize;
}

/// Handles page fault from user space.
pub(crate) fn handle_user_page_fault(
    f: &mut TrapFrame,
    exception: &CpuException,
    fault_addr: Vaddr,
) {
    // The actual user space implementation should be responsible
    // for providing mechanism to treat the 0 virtual address.
    if !(0..MAX_USERSPACE_VADDR).contains(&fault_addr) {
        panic!(
            "Cannot handle kernel page fault: {:#x?}; trapframe: {:#x?}",
            exception, f
        );
    }

    let handler = USER_PAGE_FAULT_HANDLER
        .get()
        .expect("a page fault handler is missing");

    let res = handler(exception);
    // Copying bytes by bytes can recover directly
    // if handling the page fault successfully.
    if res.is_ok() {
        return;
    }

    // Use the exception table to recover to normal execution.
    let inst_addr = f.instruction_pointer();
    if let Some(new_addr) = ExTable::find_recovery_inst_addr(inst_addr) {
        f.set_instruction_pointer(new_addr);
    } else {
        panic!("Cannot handle user page fault; trapframe: {:#x?}", f);
    }
}
