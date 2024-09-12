// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use aster_rights::Full;
use ostd::{cpu::*, mm::VmSpace};

use crate::{
    prelude::*,
    process::signal::signals::fault::FaultSignal,
    vm::{page_fault_handler::PageFaultHandler, vmar::Vmar},
};

/// We can't handle most exceptions, just send self a fault signal before return to user space.
pub fn handle_exception(ctx: &Context, context: &UserContext) {
    let trap_info = context.trap_information();
    let exception = CpuException::to_cpu_exception(trap_info.id as u16).unwrap();
    log_trap_info(exception, trap_info);

    match *exception {
        PAGE_FAULT => {
            if handle_page_fault_from_vmar(ctx.process.root_vmar(), trap_info).is_err() {
                generate_fault_signal(trap_info, ctx);
            }
        }
        _ => {
            // We current do nothing about other exceptions
            generate_fault_signal(trap_info, ctx);
        }
    }
}

/// Handles the page fault occurs in the input `VmSpace`.
pub(crate) fn handle_page_fault_from_vm_space(
    vm_space: &VmSpace,
    trap_info: &CpuExceptionInfo,
) -> core::result::Result<(), ()> {
    let current = current!();
    let root_vmar = current.root_vmar();

    // If page is not present or due to write access, we should ask the vmar try to commit this page
    debug_assert_eq!(
        Arc::as_ptr(root_vmar.vm_space()),
        vm_space as *const VmSpace
    );

    handle_page_fault_from_vmar(root_vmar, trap_info)
}

/// Handles the page fault occurs in the input `Vmar`.
pub(crate) fn handle_page_fault_from_vmar(
    root_vmar: &Vmar<Full>,
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
        if let Err(e) = root_vmar.handle_page_fault(page_fault_addr, not_present, write) {
            warn!(
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
fn generate_fault_signal(trap_info: &CpuExceptionInfo, ctx: &Context) {
    let signal = FaultSignal::new(trap_info);
    ctx.posix_thread.enqueue_signal(Box::new(signal));
}

macro_rules! log_trap_common {
    ($exception_name: ident, $trap_info: ident) => {
        trace!(
            "[Trap][{}][err = {}]",
            stringify!($exception_name),
            $trap_info.error_code
        )
    };
}

fn log_trap_info(exception: &CpuException, trap_info: &CpuExceptionInfo) {
    match *exception {
        DIVIDE_BY_ZERO => log_trap_common!(DIVIDE_BY_ZERO, trap_info),
        DEBUG => log_trap_common!(DEBUG, trap_info),
        NON_MASKABLE_INTERRUPT => log_trap_common!(NON_MASKABLE_INTERRUPT, trap_info),
        BREAKPOINT => log_trap_common!(BREAKPOINT, trap_info),
        OVERFLOW => log_trap_common!(OVERFLOW, trap_info),
        BOUND_RANGE_EXCEEDED => log_trap_common!(BOUND_RANGE_EXCEEDED, trap_info),
        INVALID_OPCODE => log_trap_common!(INVALID_OPCODE, trap_info),
        DEVICE_NOT_AVAILABLE => log_trap_common!(DEVICE_NOT_AVAILABLE, trap_info),
        DOUBLE_FAULT => log_trap_common!(DOUBLE_FAULT, trap_info),
        COPROCESSOR_SEGMENT_OVERRUN => log_trap_common!(COPROCESSOR_SEGMENT_OVERRUN, trap_info),
        INVALID_TSS => log_trap_common!(INVALID_TSS, trap_info),
        SEGMENT_NOT_PRESENT => log_trap_common!(SEGMENT_NOT_PRESENT, trap_info),
        STACK_SEGMENT_FAULT => log_trap_common!(STACK_SEGMENT_FAULT, trap_info),
        GENERAL_PROTECTION_FAULT => log_trap_common!(GENERAL_PROTECTION_FAULT, trap_info),
        PAGE_FAULT => {
            trace!(
                "[Trap][{}][page fault addr = 0x{:x}, err = {}]",
                stringify!(PAGE_FAULT),
                trap_info.page_fault_addr,
                trap_info.error_code
            );
        }
        // 15 reserved
        X87_FLOATING_POINT_EXCEPTION => log_trap_common!(X87_FLOATING_POINT_EXCEPTION, trap_info),
        ALIGNMENT_CHECK => log_trap_common!(ALIGNMENT_CHECK, trap_info),
        MACHINE_CHECK => log_trap_common!(MACHINE_CHECK, trap_info),
        SIMD_FLOATING_POINT_EXCEPTION => log_trap_common!(SIMD_FLOATING_POINT_EXCEPTION, trap_info),
        VIRTUALIZATION_EXCEPTION => log_trap_common!(VIRTUALIZATION_EXCEPTION, trap_info),
        CONTROL_PROTECTION_EXCEPTION => log_trap_common!(CONTROL_PROTECTION_EXCEPTION, trap_info),
        HYPERVISOR_INJECTION_EXCEPTION => {
            log_trap_common!(HYPERVISOR_INJECTION_EXCEPTION, trap_info)
        }
        VMM_COMMUNICATION_EXCEPTION => log_trap_common!(VMM_COMMUNICATION_EXCEPTION, trap_info),
        SECURITY_EXCEPTION => log_trap_common!(SECURITY_EXCEPTION, trap_info),
        _ => {
            info!(
                "[Trap][Unknown trap type][id = {}, err = {}]",
                trap_info.id, trap_info.error_code
            );
        }
    }
}
