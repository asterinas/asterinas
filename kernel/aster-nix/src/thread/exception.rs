// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use ostd::{cpu::*, mm::VmSpace};

use crate::{
    arch::exception::log_trap_info,
    prelude::*,
    process::signal::signals::fault::FaultSignal,
    vm::{page_fault_handler::PageFaultHandler, perms::VmPerms},
};

pub trait PageFaultContext {
    /// Whether it's a page fault
    fn is_page_fault(&self) -> bool;

    /// The virtual address where a page fault occurred.
    fn page_fault_address(&self) -> Vaddr;

    /// The VmPerms required by the memory operation that causes page fault.
    /// For example, a "store" operation may require `VmPerms::WRITE`.
    fn page_fault_required_perms(&self) -> VmPerms;
}

/// We can't handle most exceptions, just send self a fault signal before return to user space.
pub fn handle_exception(context: &UserContext) {
    let trap_info = context.trap_information();
    log_trap_info(trap_info);

    if trap_info.is_page_fault() {
        let current = current!();
        let root_vmar = current.root_vmar();

        if handle_page_fault(root_vmar.vm_space(), trap_info).is_ok() {
            return;
        }
    }

    generate_fault_signal(trap_info);
}

/// Handles the page fault occurs in the input `VmSpace`.
pub(crate) fn handle_page_fault(
    vm_space: &VmSpace,
    trap_info: &CpuExceptionInfo,
) -> core::result::Result<(), ()> {
    let page_fault_addr = trap_info.page_fault_address();
    let perms = trap_info.page_fault_required_perms();

    // If page is not present or due to write access, we should ask the vmar try to commit this page
    let current = current!();
    let root_vmar = current.root_vmar();

    debug_assert_eq!(
        Arc::as_ptr(root_vmar.vm_space()),
        vm_space as *const VmSpace
    );

    if let Err(e) = root_vmar.handle_page_fault(page_fault_addr, perms) {
        error!(
            "page fault handler failed: addr: 0x{:x}, err: {:?}",
            page_fault_addr, e
        );
        return Err(());
    }
    Ok(())
}

/// generate a fault signal for current process.
fn generate_fault_signal(trap_info: &CpuExceptionInfo) {
    let current = current!();
    let signal = FaultSignal::from(trap_info);
    current.enqueue_signal(signal);
}
