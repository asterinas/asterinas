// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use aster_rights::Full;
use ostd::{cpu::*, mm::VmSpace};

use crate::{
    prelude::*,
    process::signal::signals::fault::FaultSignal,
    vm::{page_fault_handler::PageFaultHandler, perms::VmPerms, vmar::Vmar},
};

/// Page fault information converted from [`CpuExceptionInfo`].
///
/// `From<CpuExceptionInfo>` should be implemented for this struct.
/// If `CpuExceptionInfo` is a page fault, `try_from` should return `Ok(PageFaultInfo)`,
/// or `Err(())` (no error information) otherwise.
pub struct PageFaultInfo {
    /// The virtual address where a page fault occurred.
    pub address: Vaddr,

    /// The [`VmPerms`] required by the memory operation that causes page fault.
    /// For example, a "store" operation may require `VmPerms::WRITE`.
    pub required_perms: VmPerms,
}

/// We can't handle most exceptions, just send self a fault signal before return to user space.
pub fn handle_exception(ctx: &Context, context: &UserContext) {
    let trap_info = context.trap_information();
    log_trap_info(trap_info);

    if let Ok(page_fault_info) = PageFaultInfo::try_from(trap_info) {
        if handle_page_fault_from_vmar(ctx.process.root_vmar(), &page_fault_info).is_ok() {
            return;
        }
    }

    generate_fault_signal(trap_info, ctx);
}

/// Handles the page fault occurs in the input `VmSpace`.
pub(crate) fn handle_page_fault_from_vm_space(
    vm_space: &VmSpace,
    page_fault_info: &PageFaultInfo,
) -> core::result::Result<(), ()> {
    let current = current!();
    let root_vmar = current.root_vmar();

    // If page is not present or due to write access, we should ask the vmar try to commit this page
    debug_assert_eq!(
        Arc::as_ptr(root_vmar.vm_space()),
        vm_space as *const VmSpace
    );

    handle_page_fault_from_vmar(root_vmar, page_fault_info)
}

/// Handles the page fault occurs in the input `Vmar`.
pub(crate) fn handle_page_fault_from_vmar(
    root_vmar: &Vmar<Full>,
    page_fault_info: &PageFaultInfo,
) -> core::result::Result<(), ()> {
    if let Err(e) = root_vmar.handle_page_fault(page_fault_info) {
        warn!(
            "page fault handler failed: addr: 0x{:x}, err: {:?}",
            page_fault_info.address, e
        );
        return Err(());
    }
    Ok(())
}

/// generate a fault signal for current process.
fn generate_fault_signal(trap_info: &CpuExceptionInfo, ctx: &Context) {
    let signal = FaultSignal::from(trap_info);
    ctx.posix_thread.enqueue_signal(Box::new(signal));
}

fn log_trap_info(trap_info: &CpuExceptionInfo) {
    if let Ok(page_fault_info) = PageFaultInfo::try_from(trap_info) {
        trace!(
            "[Trap][PAGE_FAULT][page fault addr = 0x{:x}, err = {}]",
            trap_info.page_fault_addr,
            trap_info.error_code
        );
    } else {
        let exception = trap_info.cpu_exception();
        trace!("[Trap][{exception:?}][err = {}]", trap_info.error_code)
    }
}
