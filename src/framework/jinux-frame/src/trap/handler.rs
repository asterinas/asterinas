use crate::task::{
    context_switch, get_idle_task_cx_ptr, Task, TaskContext, SWITCH_TO_USER_SPACE_TASK,
};

use super::{irq::IRQ_LIST, *};

#[no_mangle]
pub(crate) extern "C" fn syscall_handler(f: &mut SyscallFrame) -> isize {
    let r = &f.caller;
    let current = Task::current();
    current.inner_exclusive_access().is_from_trap = false;
    *current.syscall_frame() = *SWITCH_TO_USER_SPACE_TASK.get().syscall_frame();
    unsafe {
        context_switch(
            get_idle_task_cx_ptr() as *mut TaskContext,
            &Task::current().inner_ctx() as *const TaskContext,
        )
    }
    -1
}

#[no_mangle]
pub(crate) extern "C" fn trap_handler(f: &mut TrapFrame) {
    if !is_from_kernel(f.cs) {
        let current = Task::current();
        current.inner_exclusive_access().is_from_trap = true;
        *current.trap_frame() = *SWITCH_TO_USER_SPACE_TASK.trap_frame();
        if is_cpu_fault(current.trap_frame()) {
            // if is cpu fault, we will pass control to trap handler in jinux std
            unsafe {
                context_switch(
                    get_idle_task_cx_ptr() as *mut TaskContext,
                    &Task::current().inner_ctx() as *const TaskContext,
                )
            }
        } else {
            let irq_line = IRQ_LIST.get(f.id as usize).unwrap();
            let callback_functions = irq_line.callback_list();
            for callback_function in callback_functions.iter() {
                callback_function.call(f);
            }
        }
    } else {
        if is_cpu_fault(f) {
            panic!("cannot handle kernel cpu fault now");
        }
        let irq_line = IRQ_LIST.get(f.id as usize).unwrap();
        let callback_functions = irq_line.callback_list();
        for callback_function in callback_functions.iter() {
            callback_function.call(f);
        }
    }
}

fn is_from_kernel(cs: u64) -> bool {
    if cs & 0x3 == 0 {
        true
    } else {
        false
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
    match trap_frame.id {
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
