// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use ostd::{cpu::*, mm::VmSpace};

use crate::{
    prelude::*,
    process::signal::signals::fault::FaultSignal,
    vm::{page_fault_handler::PageFaultHandler, perms::VmPerms},
};

/// This trait is implemented by [`CpuExceptionInfo`] which can tell the information of a page
/// fault.
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
    let signal = FaultSignal::new(trap_info);
    current.enqueue_signal(signal);
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

fn log_trap_info(trap_info: &CpuExceptionInfo) {
    match *trap_info.cpu_exception() {
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
        INVAILD_TSS => log_trap_common!(INVAILD_TSS, trap_info),
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
