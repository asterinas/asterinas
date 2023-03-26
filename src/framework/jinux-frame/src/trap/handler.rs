use super::{irq::IRQ_LIST, *};
use trapframe::TrapFrame;

/// Only from kernel
#[no_mangle]
extern "sysv64" fn trap_handler(f: &mut TrapFrame) {
    if is_cpu_fault(f) {
        panic!("cannot handle kernel cpu fault now, information:{:#x?}", f);
    }
    call_irq_callback_functions(f);
}

pub(crate) fn call_irq_callback_functions(f: &mut TrapFrame) {
    let irq_line = IRQ_LIST.get(f.trap_num as usize).unwrap();
    let callback_functions = irq_line.callback_list();
    for callback_function in callback_functions.iter() {
        callback_function.call(f);
    }
    if f.trap_num >= 0x20 {
        crate::arch::interrupts_ack();
    }
}

/// As Osdev Wiki defines(https://wiki.osdev.org/Exceptions):
/// CPU exceptions are classified as:

/// Faults: These can be corrected and the program may continue as if nothing happened.
/// Traps: Traps are reported immediately after the execution of the trapping instruction.
/// Aborts: Some severe unrecoverable error.

/// This function will determine a trap is a CPU faults.
/// We will pass control to jinux-std if the trap is **faults**.
pub fn is_cpu_fault(trap_frame: &TrapFrame) -> bool {
    match trap_frame.trap_num {
        DIVIDE_BY_ZERO
        | DEBUG
        | BOUND_RANGE_EXCEEDED
        | INVALID_OPCODE
        | DEVICE_NOT_AVAILABLE
        | INVAILD_TSS
        | SEGMENT_NOT_PRESENT
        | STACK_SEGMENT_FAULT
        | GENERAL_PROTECTION_FAULT
        | PAGE_FAULT
        | X87_FLOATING_POINT_EXCEPTION
        | ALIGNMENT_CHECK
        | SIMD_FLOATING_POINT_EXCEPTION
        | VIRTUALIZATION_EXCEPTION
        | CONTROL_PROTECTION_EXCEPTION
        | HYPERVISOR_INJECTION_EXCEPTION
        | VMM_COMMUNICATION_EXCEPTION
        | SECURITY_EXCEPTION => true,
        _ => false,
    }
}
