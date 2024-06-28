// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use ostd::{cpu::*, mm::VmSpace};

use crate::{
    prelude::*, process::signal::signals::fault::FaultSignal,
    vm::page_fault_handler::PageFaultHandler,
};

/// We can't handle most exceptions, just send self a fault signal before return to user space.
pub fn handle_exception(context: &UserContext) {
    let trap_info = context.trap_information();
    let exception = CpuException::from_num(trap_info.id as u16);
    log_trap_info(&exception, trap_info);
    let current = current!();
    let root_vmar = current.root_vmar();

    match exception {
        CpuException::PageFault => {
            if handle_page_fault(root_vmar.vm_space(), trap_info).is_err() {
                generate_fault_signal(trap_info);
            }
        }
        _ => {
            // We current do nothing about other exceptions
            generate_fault_signal(trap_info);
        }
    }
}

/// Handles the page fault occurs in the input `VmSpace`.
pub(crate) fn handle_page_fault(
    vm_space: &VmSpace,
    trap_info: &CpuExceptionInfo,
) -> core::result::Result<(), ()> {
    const PAGE_NOT_PRESENT_ERROR_MASK: usize = 0x1 << 0;
    const WRITE_ACCESS_MASK: usize = 0x1 << 1;
    let page_fault_addr = trap_info.page_fault_addr as Vaddr;
    trace!(
        "page fault error code: 0x{:x}, Page fault address: 0x{:x}",
        trap_info.error_code,
        page_fault_addr
    );
    let not_present = trap_info.error_code & PAGE_NOT_PRESENT_ERROR_MASK == 0;
    let write = trap_info.error_code & WRITE_ACCESS_MASK != 0;
    if not_present || write {
        // If page is not present or due to write access, we should ask the vmar try to commit this page
        let current = current!();
        let root_vmar = current.root_vmar();

        debug_assert_eq!(
            Arc::as_ptr(root_vmar.vm_space()),
            vm_space as *const VmSpace
        );

        if let Err(e) = root_vmar.handle_page_fault(page_fault_addr, not_present, write) {
            error!(
                "page fault handler failed: addr: 0x{:x}, err: {:?}",
                page_fault_addr, e
            );
            return Err(());
        }
        Ok(())
    } else {
        // Otherwise, the page fault cannot be handled
        Err(())
    }
}

/// generate a fault signal for current process.
fn generate_fault_signal(trap_info: &CpuExceptionInfo) {
    let current = current!();
    let signal = FaultSignal::new(trap_info);
    current.enqueue_signal(signal);
}

fn log_trap_info(exception: &CpuException, trap_info: &CpuExceptionInfo) {
    match exception {
        CpuException::PageFault => {
            trace!(
                "[Trap][{:?}][page fault addr = 0x{:x}, err = {}]",
                exception,
                trap_info.page_fault_addr,
                trap_info.error_code
            );
        }
        _ => {
            trace!("[Trap][{:?}][err = {}]", exception, trap_info.error_code);
        }
    }
}
