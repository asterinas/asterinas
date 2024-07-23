// SPDX-License-Identifier: MPL-2.0

use ostd::cpu::*;

use crate::prelude::*;

macro_rules! log_trap_common {
    ($exception_name: ident, $trap_info: ident) => {
        trace!(
            "[Trap][{}][err = {}]",
            stringify!($exception_name),
            $trap_info.error_code
        )
    };
}

pub fn log_trap_info(trap_info: &CpuExceptionInfo) {
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
